use matrust::{
    Engine,
    help::{self, PathContext},
};
use std::fs;

fn output(engine: &mut Engine, source: &str) -> String {
    engine.execute(source).unwrap().output.join("\n")
}

fn help(source: &str) -> String {
    output(&mut Engine::default(), source)
}

#[test]
fn help_alone_prints_the_overview_with_a_pointer_to_function_help() {
    for command in ["help", "help;", "doc", "help()"] {
        let text = help(command);
        assert_eq!(text, matrust::engine::HELP, "{command}");
        assert!(text.starts_with("Matrices:"));
        assert!(text.ends_with("\nType help <name> for a function, for example help plot."));
    }
}

#[test]
fn page_found_prints_the_page_body() {
    let sin = help("help sin");
    assert_eq!(
        sin,
        "Sine of argument in radians.\nSyntax: y = sin(x)\nElementwise sine; complex input is supported.\nExample: sin(pi/2)   % 1\nExample: sin(1+2i)   % 3.1658 + 1.9596i\nDifferences from MATLAB: logical input is not supported yet."
    );
    // Command, function and string forms, and doc, all agree.
    for source in [
        "help('sin')",
        "help(\"sin\")",
        "doc sin",
        "name='sin';help(name)",
    ] {
        assert_eq!(help(source), sin, "{source}");
    }
    for name in ["sort", "zeros", "plot", "help"] {
        let page = help(&format!("help {name}"));
        assert_eq!(Some(page.lines().next().unwrap()), help::summary(name));
        assert!(page.contains("Syntax: "), "{name}");
        assert!(page.contains("Differences from MATLAB:"), "{name}");
        assert!(!page.contains("## "), "{name}");
    }
    assert_eq!(help::summary("sort"), Some("Sort array elements."));
    assert!(matrust::session::FUNCTIONS.contains(&"doc"));
    // Every supported name has a page now.
    for name in [
        "eps", "Inf", "NaN", "cell", "char", "class", "gca", "get", "set",
    ] {
        let page = help(&format!("help {name}"));
        assert_eq!(Some(page.lines().next().unwrap()), help::summary(name));
    }
}

#[test]
fn supported_matlab_only_and_unknown_names_are_told_apart() {
    assert!(help("help cumsum").starts_with("Cumulative sum."));
    assert_eq!(
        help("help uicontrol"),
        "uicontrol is a MATLAB function that MatRust does not support yet."
    );
    assert_eq!(help("doc cellfun"), help("help cellfun"));
    // The reference's first lines, then MatRust's own pointer.
    let not_found =
        |name: &str| format!("\n{name} not found.\n\nType help to list the supported functions.");
    assert_eq!(help("help nonexistentthing"), not_found("nonexistentthing"));
    assert_eq!(help("help qqqzzz"), not_found("qqqzzz"));
    // Names are case-sensitive; a supported spelling is suggested after it.
    assert_eq!(help("help Sin"), not_found("Sin") + "\nDid you mean sin?");
    assert_eq!(
        help("help LINSPACE"),
        not_found("LINSPACE") + "\nDid you mean linspace?"
    );
}

#[test]
fn help_rejects_unsupported_forms_explicitly() {
    let mut engine = Engine::default();
    let error = engine.execute("help sin cos").unwrap_err();
    assert!(
        error.contains("help: more than one name is not supported yet"),
        "{error}"
    );
    let error = engine.execute("help toolbox/sin").unwrap_err();
    assert!(error.contains("is not supported yet"), "{error}");
    let error = engine.execute("help(3)").unwrap_err();
    assert!(error.contains("must be text"), "{error}");
    let error = engine.execute("t = help('sin');").unwrap_err();
    assert!(
        error.contains("help: this call form is not supported yet"),
        "{error}"
    );
    // A variable named help keeps ordinary indexing.
    engine.execute("help = [4 5 6]; x = help(2);").unwrap();
    assert_eq!(engine.variables["x"].data(), &[5.]);
}

#[test]
fn user_functions_print_their_leading_comment_block() {
    let directory = tempfile::tempdir().unwrap();
    let files = [
        (
            "addme.mr",
            "function y = addme(a, b)\n%ADDME  Add two values.\n%   y = addme(a, b) returns a + b.\n\n% Not help: after a blank line.\ny = a + b;\nend\n",
        ),
        (
            "legacy.m",
            "% LEGACY  Old style help above the function line.\r\n%   Second line.\r\nfunction legacy()\r\n% Not help: the block above wins.\r\nend\r\n",
        ),
        (
            "bare.m",
            "function y = bare(x)\ny = x;\n% Trailing comment, not help.\nend\n",
        ),
        (
            "continued.mr",
            "function [a, b] = continued(x, ...\n    y)\n%CONTINUED  Signature over two lines.\na = x; b = y;\nend\n",
        ),
        ("script.m", "\n\n%SCRIPT  A script's top comment.\nx = 1;\n"),
        // .mr wins over .m, exactly as for calls.
        ("both.mr", "function both\n%BOTH  Native file.\nend\n"),
        ("both.m", "function both\n%BOTH  Legacy file.\nend\n"),
    ];
    for (name, source) in files {
        fs::write(directory.path().join(name), source).unwrap();
    }
    let mut engine = Engine::default();
    engine.set_function_directory(directory.path()).unwrap();
    assert_eq!(
        output(&mut engine, "help addme"),
        " ADDME  Add two values.\n    y = addme(a, b) returns a + b."
    );
    assert_eq!(
        output(&mut engine, "help legacy"),
        "  LEGACY  Old style help above the function line.\n    Second line."
    );
    assert_eq!(
        output(&mut engine, "help bare"),
        "\nNo help found for bare.m."
    );
    assert_eq!(
        output(&mut engine, "doc continued"),
        " CONTINUED  Signature over two lines."
    );
    assert_eq!(
        output(&mut engine, "help script"),
        " SCRIPT  A script's top comment."
    );
    assert_eq!(output(&mut engine, "help both"), " BOTH  Native file.");
    // A user file shadows a built-in page, as it shadows the built-in call.
    fs::write(
        directory.path().join("sin.m"),
        "function y = sin(x)\n%SIN  My own sine.\ny = x;\nend\n",
    )
    .unwrap();
    assert_eq!(output(&mut engine, "help sin"), " SIN  My own sine.");
    // The same text through the API the editor will use.
    assert_eq!(
        help::help_text("addme", &engine.help_path()),
        output(&mut engine, "help addme")
    );
    assert_eq!(
        help::help_text("sin", &PathContext::default()),
        help::page("sin").unwrap()
    );
}

#[test]
fn h1_extraction_rules() {
    assert_eq!(help::h1_text("%A\n%B\nx=1;").as_deref(), Some(" A\n B"));
    assert_eq!(
        help::h1_text("\u{feff}  % indented\n%second").as_deref(),
        Some("  indented\n second")
    );
    assert_eq!(help::h1_text("function f\n%{\nblock\n%}\nend"), None);
    assert_eq!(help::h1_text("x = 1; % trailing\n"), None);
    assert_eq!(
        help::h1_text("functional = 1;\n% not after a function line"),
        None
    );
    assert_eq!(
        help::h1_text("function[a]=f\n%F  Help.").as_deref(),
        Some(" F  Help.")
    );
    assert_eq!(help::h1_text(""), None);
}

#[test]
fn page_parser_tolerates_layout_variations() {
    let text = "Preamble text\n# Title\n\n## first\n\n\nFirst summary.\nSyntax: first(x)\n\n\nExtra line.\n\n## second   \r\nSecond summary.\r\n## first\nDuplicate ignored.\n## last\nLast summary.\nNo trailing newline";
    let pages = help::parse_pages(text);
    assert_eq!(pages.len(), 3);
    assert_eq!(
        pages["first"],
        "First summary.\nSyntax: first(x)\n\n\nExtra line."
    );
    assert_eq!(pages["second"], "Second summary.");
    assert_eq!(pages["last"], "Last summary.\nNo trailing newline");
    assert!(help::parse_pages("no headings at all\n").is_empty());
    let empty = help::parse_pages("## lonely\n\n");
    assert_eq!(empty["lonely"], "");
    // ### is not a page heading.
    assert_eq!(
        help::parse_pages("## a\nA.\n### b\nmore")["a"],
        "A.\n### b\nmore"
    );
}

#[test]
fn unknown_calls_say_whether_matlab_has_the_function() {
    let mut engine = Engine::default();
    engine.execute("x = 4;").unwrap();
    for (source, expected) in [
        (
            "y = uicontrol(1);",
            "'uicontrol' is a MATLAB function that MatRust does not support yet.",
        ),
        ("y = qqqzzz(1);", "Undefined function or variable 'qqqzzz'."),
        (
            "y = undefinedThing;",
            "Undefined function or variable 'undefinedThing'.",
        ),
        (
            "undefinedThing",
            "Undefined function or variable 'undefinedThing'.",
        ),
        (
            "y = uitable;",
            "'uitable' is a MATLAB function that MatRust does not support yet.",
        ),
    ] {
        let error = engine.execute(source).unwrap_err();
        assert!(error.contains(expected), "{source}: {error}");
        assert_eq!(engine.variables["x"].data(), &[4.]);
    }
    assert_eq!(
        help::unknown_function("uicontrol"),
        "'uicontrol' is a MATLAB function that MatRust does not support yet."
    );
    assert_eq!(
        help::unknown_function("qqqzzz"),
        "Undefined function or variable 'qqqzzz'."
    );
    assert!(help::is_matlab_name("uicontrol") && !help::is_matlab_name("UiControl"));
}

#[test]
fn unknown_names_with_several_outputs_get_the_same_texts() {
    let mut engine = Engine::default();
    for (source, expected) in [
        (
            "[a,b] = uicontrol(1);",
            "'uicontrol' is a MATLAB function that MatRust does not support yet.",
        ),
        (
            "[a,b] = qqqzzz(1);",
            "Undefined function or variable 'qqqzzz'.",
        ),
        ("[a,b,c] = zeros(2);", "zeros does not support 3 outputs"),
    ] {
        let error = engine.execute(source).unwrap_err();
        assert!(error.contains(expected), "{source}: {error}");
    }
}

#[test]
fn h1_ignores_ellipsis_in_a_trailing_comment() {
    assert_eq!(
        help::h1_text("function y=f(x) % example ...\n% Help.\ny=x;").as_deref(),
        Some("  Help.")
    );
    assert_eq!(
        help::h1_text("function y=f(x, ...\n  z)\n% Help.").as_deref(),
        Some("  Help.")
    );
    assert_eq!(
        help::h1_text("function y=f(x, 'a...') % c\n% Help.").as_deref(),
        Some("  Help.")
    );
}

#[test]
fn user_help_matches_matlab_reference_cases_hf1_to_hf10() {
    // Reference inputs and expected help text.
    let cases = [
        (
            "hf1",
            "function hf1\n% HF1 first line\n% second line\nend\n",
            "  HF1 first line\n  second line",
        ),
        (
            "hf2",
            "function hf2\n\n  \n% HF2 after blank\n% second\nend\n",
            "  HF2 after blank\n  second",
        ),
        (
            "hf3",
            "function hf3\n%  HF3 two spaces\n%    four spaces\n%no space\nend\n",
            "   HF3 two spaces\n     four spaces\n no space",
        ),
        (
            "hf4",
            "% HF4 above function\n% second above\nfunction hf4\nend\n",
            "  HF4 above function\n  second above",
        ),
        (
            "hf5",
            "% HF5 script comment\n% second line\nx = 1;\n",
            "  HF5 script comment\n  second line",
        ),
        (
            "hf6",
            "function hf6\n% first block\n\n% second block\nend\n",
            "  first block",
        ),
        (
            "hf7",
            "function y = hf7(x)\ny = x; % trailing\n% after code\nend\n",
            "\nNo help found for hf7.m.",
        ),
        (
            "hf8",
            "%HF8 nospace\n%\n% blank-comment above\nfunction hf8\n% inside\nend\n",
            " HF8 nospace\n \n  blank-comment above",
        ),
        (
            "hf9",
            "function hf9\n  % indented comment\n  % second indented\nend\n",
            "  indented comment\n  second indented",
        ),
        (
            "hf10",
            "function y = hf10(x) % on function line\n% next line\ny = x;\nend\n",
            "  next line",
        ),
    ];
    for extension in ["m", "mr"] {
        let directory = tempfile::tempdir().unwrap();
        for (name, source, _) in cases {
            fs::write(directory.path().join(format!("{name}.{extension}")), source).unwrap();
        }
        let mut engine = Engine::default();
        engine.set_function_directory(directory.path()).unwrap();
        for (name, _, expected) in cases {
            // The no-help text names the file that was found.
            let expected = expected.replace("hf7.m.", &format!("hf7.{extension}."));
            assert_eq!(
                output(&mut engine, &format!("help {name}")),
                expected,
                "{name}.{extension}"
            );
        }
    }
}

fn pages() -> std::collections::BTreeMap<&'static str, &'static str> {
    help::parse_pages(include_str!("../assets/help/functions.md"))
}

#[test]
fn every_page_heading_is_a_supported_function() {
    let pages = pages();
    assert!(pages.len() >= 5);
    for name in pages.keys() {
        assert!(
            matrust::session::FUNCTIONS.contains(name),
            "page {name} is not in FUNCTIONS"
        );
    }
}

#[test]
fn every_supported_function_has_a_page() {
    let pages = pages();
    let missing: Vec<_> = matrust::session::FUNCTIONS
        .iter()
        .filter(|name| !pages.contains_key(*name))
        .collect();
    assert!(missing.is_empty(), "no page for {missing:?}");
}

#[test]
fn every_page_has_a_summary_and_a_syntax_line() {
    for (name, body) in pages() {
        let mut lines = body.lines().map(str::trim_end);
        let summary = lines.next().unwrap_or("");
        assert!(
            !summary.is_empty() && !summary.starts_with("Syntax:"),
            "page {name} has no summary line"
        );
        assert!(
            lines.any(|line| line.starts_with("Syntax:")),
            "page {name} has no Syntax line"
        );
    }
}

/// Functions whose examples draw, open or write files, or need a calling
/// function (nargin, nargout). They are checked by hand with the CLI.
const NOT_RUN_IN_TESTS: &[&str] = &[
    "annotation",
    "axes",
    "axis",
    "bar",
    "csvread",
    "dlmread",
    "exportgraphics",
    "feof",
    "fgetl",
    "fgets",
    "figure",
    "fread",
    "frewind",
    "fscanf",
    "fseek",
    "ftell",
    "gca",
    "gcf",
    "get",
    "grid",
    "hold",
    "isgraphics",
    "legend",
    "load",
    "loglog",
    "nargin",
    "narginchk",
    "nargout",
    "openfig",
    "plot",
    "readmatrix",
    "run",
    "save",
    "saveas",
    "savefig",
    "scatter",
    "semilogx",
    "set",
    "sgtitle",
    "stem",
    "subplot",
    "title",
    "writematrix",
    "xlabel",
];

#[test]
fn page_examples_run_in_a_fresh_engine() {
    let mut failures = Vec::new();
    let mut ran = 0;
    for (name, body) in pages() {
        if NOT_RUN_IN_TESTS.contains(&name) {
            continue;
        }
        for line in body.lines().filter_map(|l| l.strip_prefix("Example:")) {
            let code = line.split("   %").next().unwrap().trim();
            ran += 1;
            if let Err(error) = Engine::default().execute(code) {
                failures.push(format!("{name}: {code}: {error}"));
            }
        }
    }
    assert!(ran > 150, "only {ran} examples ran");
    assert!(
        failures.is_empty(),
        "examples failed:\n{}",
        failures.join("\n")
    );
}

#[test]
fn pages_have_no_internal_references() {
    const CASE_SENSITIVE: &[&str] = &[
        "tests/",
        "fixtures",
        "scratchpad",
        concat!("research", "/"),
        ".rs",
        "cplx1",
        "cplx2",
        "cplx3",
        "KNOWN",
        "HARNESS",
    ];
    const LOWERCASE: &[&str] = &["corpus", "replay", "oracle"];
    let text = include_str!("../assets/help/functions.md");
    let offending: Vec<_> = text
        .lines()
        .enumerate()
        .filter(|(_, line)| {
            let lower = line.to_lowercase();
            CASE_SENSITIVE.iter().any(|term| line.contains(term))
                || LOWERCASE.iter().any(|term| lower.contains(term))
        })
        .map(|(index, _)| index + 1)
        .collect();
    assert!(
        offending.is_empty(),
        "internal references in assets/help/functions.md on lines {offending:?}"
    );
}
