//! `format shortG|longG|shortE|longE|shortEng|longEng` in every spelling, and `close all force|hidden`
//! (fix line 11b1).
//!
//! Number text follows the public MathWorks `format` reference page: shortG and
//! longG are fixed point or `e` form, whichever is more compact, with 5 and 15
//! significant digits (7 for single); shortE and longE are `e` form with 4 and 15
//! decimals (7 for single); integer-valued values of at most 9 digits never show
//! in `e` form. Values agree with GNU Octave 10 (`tool/ref-octave.sh`) except
//! where the page differs: Octave's long g keeps 16 digits (`3.141592653589793`,
//! single `3.1415927`) and shows integer values in `e` form (`1e+05`,
//! `1.0000e+05`). Column widths (13 short, 26 long, the `e` form widths of
//! `format short|long`) and complex `%g` spacing are not reference-checked.
use matrust::Engine;
use matrust::display_format::{DisplayFormat, Notation};

fn console(e: &mut Engine, source: &str) -> String {
    let result = e
        .execute(source)
        .unwrap_or_else(|error| panic!("{source}: {error}"));
    result
        .output
        .iter()
        .map(|entry| format!("{entry}\n"))
        .collect()
}

/// `name = value` text under `format compact`, without the header line.
fn shown(e: &mut Engine, assignment: &str) -> String {
    let text = console(e, assignment);
    let (_, body) = text.split_once('\n').expect("an echo");
    body.trim_end_matches('\n').to_owned()
}

fn disp(e: &mut Engine, value: &str) -> String {
    console(e, &format!("disp({value})")).trim().to_owned()
}

fn engine(style: &str) -> Engine {
    let mut e = Engine::default();
    console(&mut e, &format!("format {style}"));
    console(&mut e, "format compact");
    e
}

const SPELLINGS: &[(&str, &[&str])] = &[
    (
        "3.14159265358979",
        &[
            "format long g",
            "format longG",
            "format longg",
            "format LONG G",
            "format Long g",
            "format('long','g')",
            "format('long', 'G')",
            "format('longG')",
            "format(\"long g\")",
            "format(\"longG\")",
        ],
    ),
    (
        "3.1416",
        &[
            "format short g",
            "format shortG",
            "format shortg",
            "format('short','g')",
            "format('shortG')",
        ],
    ),
    (
        "3.141592653589793e+00",
        &[
            "format long e",
            "format longE",
            "format longe",
            "format('long','e')",
            "format('longE')",
        ],
    ),
    (
        "3.1416e+00",
        &[
            "format short e",
            "format shortE",
            "format SHORTE",
            "format('short','e')",
            "format('shortE')",
        ],
    ),
];

#[test]
fn every_spelling_selects_its_style() {
    let mut e = Engine::default();
    for (expected, spellings) in SPELLINGS {
        for spelling in *spellings {
            console(&mut e, spelling);
            assert_eq!(disp(&mut e, "pi"), *expected, "{spelling}");
            // `format` alone resets to short and loose.
            console(&mut e, "format");
            assert_eq!(e.display, DisplayFormat::default(), "{spelling}");
            assert_eq!(disp(&mut e, "pi"), "3.1416");
        }
    }
    console(&mut e, "format long");
    assert_eq!(disp(&mut e, "pi"), "3.141592653589793");
    console(&mut e, "format longG; format short");
    assert_eq!(e.display.notation, Notation::Fixed);
    assert_eq!(disp(&mut e, "pi"), "3.1416");
    console(&mut e, "format long e; format default");
    assert_eq!(e.display, DisplayFormat::default());
}

#[test]
fn compact_and_loose_combine_with_every_style() {
    let mut e = Engine::default();
    console(&mut e, "format long g");
    assert_eq!(
        console(&mut e, "x = pi"),
        "\nx =\n\n          3.14159265358979\n\n"
    );
    console(&mut e, "format compact");
    assert_eq!(
        console(&mut e, "x = pi"),
        "x =\n          3.14159265358979\n"
    );
    console(&mut e, "format short e");
    assert_eq!(console(&mut e, "x = pi"), "x =\n   3.1416e+00\n");
    assert!(e.display.compact);
    console(&mut e, "format loose");
    assert_eq!(console(&mut e, "x = pi"), "\nx =\n\n   3.1416e+00\n\n");
    assert_eq!(e.display.notation, Notation::Scientific);
    console(&mut e, "format compact; format");
    assert_eq!(console(&mut e, "x = pi"), "\nx =\n\n    3.1416\n\n");
}

#[test]
fn unknown_options_name_the_choices() {
    let mut e = Engine::default();
    console(&mut e, "format long g");
    for bad in [
        "format bank",
        "format long x",
        "format g long",
        "format short long",
        "format long compact",
        "format long g compact",
        "format longGG",
        "format('')",
        "format(1)",
    ] {
        let error = e.execute(bad).expect_err(bad);
        assert!(
            error.contains("format:")
                && (error.contains("shortG") || error.contains("text") || error.contains("two")),
            "{bad}: {error}"
        );
        if !error.contains("text") && !error.contains("at most") {
            assert!(
                error.contains("compact") && error.contains("longE"),
                "{bad}: {error}"
            );
        }
    }
    // A rejected option leaves the format as it was.
    assert_eq!(disp(&mut e, "pi"), "3.14159265358979");
}

#[test]
fn short_g_values() {
    let mut e = engine("short g");
    for (value, expected) in [
        ("pi", "       3.1416"),
        ("-pi", "      -3.1416"),
        ("1e5", "      100000"),
        ("7", "     7"),
        ("-0", "     0"),
        ("123456.7", "   1.2346e+05"),
        ("99999.4", "        99999"),
        ("99999.5", "        1e+05"),
        ("1000.5", "       1000.5"),
        ("0.0001", "       0.0001"),
        ("-0.000123456", "  -0.00012346"),
        ("1.5e-7", "      1.5e-07"),
        ("1e20", "        1e+20"),
        ("0.1+0.2", "          0.3"),
        ("NaN", "   NaN"),
        ("-Inf", "  -Inf"),
        ("[]", "     []"),
        ("zeros(0,3)", "  0\u{d7}3 empty double matrix"),
        ("[1 2 3]", "     1     2     3"),
        ("[1 1000.5 1e-5]", "            1       1000.5        1e-05"),
        (
            "[1 2;3 4.5]",
            "            1            2\n            3          4.5",
        ),
        ("[NaN Inf -1.5]", "          NaN          Inf         -1.5"),
        (
            "[1e-3 1;100 1e6]",
            "        0.001            1\n          100        1e+06",
        ),
        ("[1e10 1]", "        1e+10            1"),
        ("[0 1.5]", "            0          1.5"),
        ("3+4i", "            3 +          4i"),
        ("-1.5-0.25i", "         -1.5 -       0.25i"),
        (
            "[1+2i 3.5-4i]",
            "            1 +          2i          3.5 -          4i",
        ),
        ("complex(NaN,Inf)", "          NaN +        Infi"),
        ("single(pi)", "      3.1416"),
        ("single([1 2.5])", "           1         2.5"),
        ("{pi, 2}", "    [3.1416]    [2]"),
    ] {
        assert_eq!(shown(&mut e, &format!("x = {value}")), expected, "{value}");
    }
    assert_eq!(disp(&mut e, "1e5"), "100000");
    assert_eq!(
        disp(&mut e, "[1 1000.5 1e-5]"),
        "1       1000.5        1e-05"
    );
    assert_eq!(disp(&mut e, "3+4i"), "3 +          4i");
    assert_eq!(disp(&mut e, "single(0.1)"), "0.1");
    assert_eq!(shown(&mut e, "s.a = 0.1+0.2"), "    a: 0.3");
}

#[test]
fn long_g_values() {
    let mut e = engine("long g");
    for (value, expected) in [
        ("pi", "          3.14159265358979"),
        ("-pi", "         -3.14159265358979"),
        ("0.1+0.2", "                       0.3"),
        ("1e5", "      100000"),
        ("123456789.5", "               123456789.5"),
        ("12345.678901234567", "          12345.6789012346"),
        ("1e20", "                     1e+20"),
        ("1.5e-7", "                   1.5e-07"),
        ("NaN", "   NaN"),
        ("[1 2 3]", "     1     2     3"),
        (
            "[pi; -exp(1)]",
            "          3.14159265358979\n         -2.71828182845905",
        ),
        (
            "[1 1000.5 1e-5]",
            "                         1                    1000.5                     1e-05",
        ),
        (
            "3+4i",
            "                          3 +                     4i",
        ),
        ("single(pi)", "        3.141593"),
    ] {
        assert_eq!(shown(&mut e, &format!("x = {value}")), expected, "{value}");
    }
    assert_eq!(disp(&mut e, "-exp(1)"), "-2.71828182845905");
    assert_eq!(disp(&mut e, "single(1/3)"), "0.3333333");
}

#[test]
fn short_e_values() {
    let mut e = engine("short e");
    for (value, expected) in [
        ("pi", "   3.1416e+00"),
        ("-pi", "  -3.1416e+00"),
        ("1e5", "      100000"),
        ("1e20", "   1.0000e+20"),
        ("0.5", "   5.0000e-01"),
        ("Inf", "   Inf"),
        ("[]", "     []"),
        ("[1 2 3]", "     1     2     3"),
        ("[1 1000.5 1e-5]", "   1.0000e+00   1.0005e+03   1.0000e-05"),
        (
            "[0 2;3 4.5]",
            "            0   2.0000e+00\n   3.0000e+00   4.5000e+00",
        ),
        ("[NaN Inf -1.5]", "          NaN          Inf  -1.5000e+00"),
        ("3+4i", "   3.0000e+00 + 4.0000e+00i"),
        (
            "[1+2i 3.5-4i]",
            "   1.0000e+00 + 2.0000e+00i   3.5000e+00 - 4.0000e+00i",
        ),
        ("single(pi)", "  3.1416e+00"),
    ] {
        assert_eq!(shown(&mut e, &format!("x = {value}")), expected, "{value}");
    }
    assert_eq!(disp(&mut e, "[0.5 0.25]"), "5.0000e-01   2.5000e-01");
    assert_eq!(shown(&mut e, "c = {0.5}"), "    [5.0000e-01]");
}

#[test]
fn long_e_values() {
    let mut e = engine("long e");
    for (value, expected) in [
        ("pi", "     3.141592653589793e+00"),
        ("-0.1", "    -1.000000000000000e-01"),
        ("42", "    42"),
        (
            "[1 1000.5 1e-5]",
            "     1.000000000000000e+00     1.000500000000000e+03     1.000000000000000e-05",
        ),
        ("[NaN -Inf 0]", "   NaN  -Inf     0"),
        (
            "3+4i",
            "      3.000000000000000e+00 + 4.000000000000000e+00i",
        ),
        ("single(pi)", "   3.1415927e+00"),
        ("single([1 2.5])", "   1.0000000e+00   2.5000000e+00"),
    ] {
        assert_eq!(shown(&mut e, &format!("x = {value}")), expected, "{value}");
    }
}

#[test]
fn close_all_force_and_hidden_close_every_figure() {
    for source in [
        "close all force",
        "close all hidden",
        "close('all','force')",
        "close('all','hidden')",
        "close all FORCE",
        "close(\"all\", \"force\")",
    ] {
        let mut e = Engine::default();
        console(&mut e, "figure; plot(1:3); figure; figure(7);");
        assert_eq!(e.figure_ids().len(), 3, "{source}");
        console(&mut e, source);
        assert!(e.figure_ids().is_empty(), "{source}: {:?}", e.figure_ids());
    }
    let mut e = Engine::default();
    console(&mut e, "figure(3); figure(4); close(3, 'force');");
    assert_eq!(e.figure_ids(), [4]);
    console(&mut e, "h = figure(5); close(h, 'force');");
    assert_eq!(e.figure_ids(), [4]);
    console(&mut e, "close(4, 'hidden');");
    assert!(e.figure_ids().is_empty());
}

#[test]
fn engineering_spellings_select_engineering_notation() {
    let mut e = Engine::default();
    for spelling in [
        "format shortEng",
        "format shorteng",
        "format SHORTENG",
        "format short eng",
        "format Short Eng",
        "format('short','eng')",
        "format('shortEng')",
        "format(\"short eng\")",
    ] {
        console(&mut e, spelling);
        assert_eq!(disp(&mut e, "12345"), "12.3450e+003", "{spelling}");
        assert_eq!(
            disp(&mut e, "0.01234567890123456"),
            "12.3457e-003",
            "{spelling}"
        );
        assert_eq!(e.display.notation, Notation::Engineering, "{spelling}");
        assert!(!e.display.long, "{spelling}");
    }
    for spelling in [
        "format longEng",
        "format longeng",
        "format LONG ENG",
        "format('long','eng')",
        "format('longEng')",
        "format(\"long eng\")",
    ] {
        console(&mut e, spelling);
        assert_eq!(
            disp(&mut e, "0.01234567890123456"),
            "12.345678901234560e-003",
            "{spelling}"
        );
        assert!(e.display.long, "{spelling}");
        assert_eq!(e.display.notation, Notation::Engineering, "{spelling}");
    }
    // `e` stays scientific. A trailing eng is not an e plus leftover letters.
    console(&mut e, "format short e");
    assert_eq!(e.display.notation, Notation::Scientific);
    assert_eq!(disp(&mut e, "pi"), "3.1416e+00");
    assert_eq!(disp(&mut e, "12345"), "12345");
    console(&mut e, "format('short','e')");
    assert_eq!(disp(&mut e, "pi"), "3.1416e+00");
}

#[test]
fn engineering_keeps_spacing_reset_and_other_engines() {
    let mut e = Engine::default();
    assert_eq!(console(&mut e, "format shortEng;"), "");
    assert_eq!(e.display.notation, Notation::Engineering);
    assert_eq!(console(&mut e, "x = 12345;"), "");
    assert_eq!(disp(&mut e, "x"), "12.3450e+003");
    console(&mut e, "format compact");
    assert_eq!(console(&mut e, "x = 12345"), "x =\n  12.3450e+003\n");
    assert!(e.display.compact);
    console(&mut e, "format loose");
    assert_eq!(console(&mut e, "x = 12345"), "\nx =\n\n  12.3450e+003\n\n");
    assert_eq!(e.display.notation, Notation::Engineering);
    assert!(!e.display.compact);
    console(&mut e, "format default");
    assert_eq!(e.display, DisplayFormat::default());
    assert_eq!(disp(&mut e, "pi"), "3.1416");

    let mut other = Engine::default();
    console(&mut e, "format longEng");
    assert_eq!(disp(&mut other, "12345"), "12345");
    assert_eq!(other.display, DisplayFormat::default());
    assert_eq!(
        disp(&mut e, "0.01234567890123456"),
        "12.345678901234560e-003"
    );
}

#[test]
fn engineering_option_errors_leave_the_format_in_place() {
    let mut e = Engine::default();
    console(&mut e, "format shortEng");
    for bad in [
        "format bank",
        "format shortEngx",
        "format eng",
        "format('short','x')",
        "format long eng extra",
        "format short e eng",
    ] {
        let error = e.execute(bad).expect_err(bad);
        assert!(
            error.contains("shortEng") || error.contains("at most") || error.contains("two"),
            "{bad}: {error}"
        );
        assert_eq!(disp(&mut e, "12345"), "12.3450e+003", "{bad}");
        assert_eq!(e.display.notation, Notation::Engineering, "{bad}");
    }
}

#[test]
fn format_help_documents_engineering_notation() {
    let mut e = Engine::default();
    let page = console(&mut e, "help format");
    assert!(page.contains("Syntax: format shortEng"), "{page}");
    assert!(page.contains("Syntax: format longEng"), "{page}");
    assert!(page.contains("12.3450e+003"), "{page}");
    assert!(
        page.contains("bank, hex, rat and + are not supported yet"),
        "{page}"
    );
    assert!(!page.contains("shortEng, longEng, bank"), "{page}");
    console(&mut e, "format shortEng; x = 12345");
    assert_eq!(disp(&mut e, "x"), "12.3450e+003");
}

#[test]
fn user_format_still_shadows_the_builtin() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("format.m"),
        "function y = format(varargin)\ny = 42;\nend\n",
    )
    .unwrap();
    let mut e = Engine::default();
    e.set_function_directory(dir.path()).unwrap();
    console(&mut e, "y = format('shortEng');");
    assert_eq!(e.variables["y"].data(), &[42.]);
    assert_eq!(e.display, DisplayFormat::default());
    console(&mut e, "format longEng");
    assert_eq!(e.display, DisplayFormat::default());
    assert_eq!(disp(&mut e, "pi"), "3.1416");

    let words = tempfile::tempdir().unwrap();
    std::fs::write(
        words.path().join("eng.m"),
        "function y = eng()\nerror('eng must not run');\nend\n",
    )
    .unwrap();
    std::fs::write(
        words.path().join("shortEng.m"),
        "function y = shortEng()\nerror('shortEng must not run');\nend\n",
    )
    .unwrap();
    let mut e = Engine::default();
    e.set_function_directory(words.path()).unwrap();
    console(&mut e, "format short eng");
    console(&mut e, "format shortEng");
    assert_eq!(disp(&mut e, "12345"), "12.3450e+003");

    let mut e = Engine::default();
    console(&mut e, "format = 7;");
    let _ = e.execute("format shortEng");
    assert_eq!(e.variables["format"].data(), &[7.]);
    assert_eq!(e.display, DisplayFormat::default());
}

#[test]
fn close_rejects_unknown_options() {
    let mut e = Engine::default();
    console(&mut e, "figure; figure;");
    for (bad, message) in [
        ("close all bogus", "unknown option 'bogus'"),
        ("close('all', 5)", "'force' or 'hidden'"),
        ("close all force now", "at most"),
    ] {
        let error = e.execute(bad).expect_err(bad);
        assert!(error.contains(message), "{bad}: {error}");
    }
    assert_eq!(e.figure_ids().len(), 2);
}
