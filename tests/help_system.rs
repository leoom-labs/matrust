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
fn help_prose_wraps_to_the_host_console_width_and_keeps_every_word() {
    let mut engine = Engine::default();
    engine.set_console_width(60);
    let text = output(&mut engine, "help");
    for line in text.lines() {
        assert!(
            line.chars().count() <= 60 || !line.trim_start().contains(' '),
            "{line}"
        );
    }
    let words = |t: &str| t.split_whitespace().map(str::to_owned).collect::<Vec<_>>();
    assert_eq!(words(&text), words(matrust::engine::HELP));
    // Without a host width (CLI, tests) help prints as stored.
    assert_eq!(help("help"), matrust::engine::HELP);
}

#[test]
fn atan2_example_comment_matches_console_output() {
    let page = help("help atan2");
    let example = page
        .lines()
        .find_map(|line| line.strip_prefix("Example: "))
        .unwrap();
    let (expression, expected) = example.split_once('%').unwrap();
    let observed = help(expression.trim());
    assert_eq!(observed.split_whitespace().last(), Some(expected.trim()));
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
fn volume_and_patch_help_describe_their_compatibility_limits() {
    for (name, limits) in [
        (
            "patch",
            &[
                "patch(x,y,c)",
                "FaceColor and EdgeColor",
                "triangles",
                "three columns",
                "one scalar per vertex",
            ][..],
        ),
        ("isosurface", &["Octave", "histogram"][..]),
        (
            "isonormals",
            &["unit length", "VertexNormals", "lighting"][..],
        ),
        (
            "isocaps",
            &["1,000,000 triangles", "100,000 patch faces"][..],
        ),
    ] {
        let page = help(&format!("help {name}"));
        let differences = page
            .lines()
            .find(|line| line.starts_with("Differences from MATLAB:"))
            .unwrap();
        for limit in limits {
            assert!(differences.contains(limit), "{name}: {limit}");
        }
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
        help("help uihtml"),
        "uihtml is a MATLAB function that MatRust does not support yet."
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
            "y = uihtml(1);",
            "'uihtml' is a MATLAB function that MatRust does not support yet.",
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
            "y = uihtml;",
            "'uihtml' is a MATLAB function that MatRust does not support yet.",
        ),
    ] {
        let error = engine.execute(source).unwrap_err();
        assert!(error.contains(expected), "{source}: {error}");
        assert_eq!(engine.variables["x"].data(), &[4.]);
    }
    assert_eq!(
        help::unknown_function("uihtml"),
        "'uihtml' is a MATLAB function that MatRust does not support yet."
    );
    assert_eq!(
        help::unknown_function("qqqzzz"),
        "Undefined function or variable 'qqqzzz'."
    );
    assert!(help::is_matlab_name("uihtml") && !help::is_matlab_name("UiHtml"));
}

#[test]
fn unknown_names_with_several_outputs_get_the_same_texts() {
    let mut engine = Engine::default();
    for (source, expected) in [
        (
            "[a,b] = uihtml(1);",
            "'uihtml' is a MATLAB function that MatRust does not support yet.",
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
            matrust::session::FUNCTIONS.contains(name)
                || OBJECT_TYPES_SINCE_V012.contains(name)
                || OBJECT_MEMBERS_SINCE_V012.contains(name)
                || APP_LAUNCHERS_SINCE_V012.contains(name)
                || relhelp_h3::METHOD_PAGES.contains(name),
            "page {name} is not in FUNCTIONS or the v0.1.3 object/member/app lists"
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
    "print",
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
    // File examples; tests/readers_more.rs runs them in a temporary folder.
    "readcell",
    "writecell",
    "xlsread",
    "readtable",
    "writetable",
    // tests/import_options.rs runs these in a temporary folder.
    "detectImportOptions",
    "setvartype",
    "getvaropts",
    "setvaropts",
    "preview",
    "readtimetable",
    // tests/web_xlswrite.rs runs these against a temporary folder and a local server.
    "xlswrite",
    "webread",
    "websave",
    // tests/hdf5_netcdf.rs runs these against fixture files.
    "h5read",
    "h5create",
    "h5write",
    "ncread",
    // Dialogs need the desktop app; tests/gui_dialogs.rs answers them from a script.
    "uigetfile",
    "uiputfile",
    "uigetdir",
    "inputdlg",
    "questdlg",
    "listdlg",
    "msgbox",
    "errordlg",
    "warndlg",
    "helpdlg",
    "waitbar",
    "uiwait",
    // edit needs the desktop app; tests/file_builtins.rs tests it.
    "edit",
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
            // File examples using pwd must stay outside the checkout.
            let directory = tempfile::tempdir().unwrap();
            let mut engine = Engine::default();
            engine.set_function_directory(directory.path()).unwrap();
            engine
                .execute("setenv('MATRUST_AUDIO_DEVICE','null');")
                .unwrap();
            if let Err(error) = engine.execute(code) {
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

/// Functions added to `matrust::session::FUNCTIONS` since v0.1.2 (commit b02cd3e3).
const FUNCTIONS_SINCE_V012: &[&str] = &[
    "arguments",
    "mustBePositive",
    "mustBeNonnegative",
    "mustBeNegative",
    "mustBeNonpositive",
    "mustBeNonzero",
    "mustBeFinite",
    "mustBeNonNan",
    "mustBeInteger",
    "mustBeReal",
    "mustBeNumeric",
    "mustBeNumericOrLogical",
    "mustBeNonempty",
    "mustBeMember",
    "mustBeInRange",
    "mustBeGreaterThan",
    "mustBeGreaterThanOrEqual",
    "mustBeLessThan",
    "mustBeLessThanOrEqual",
    "mustBeText",
    "mustBeTextScalar",
    "mustBeNonzeroLengthText",
    "mustBeFile",
    "mustBeFolder",
    "mustBeVector",
    "mustBeRow",
    "mustBeColumn",
    "mustBeScalarOrEmpty",
    "mustBeA",
    "mustBeUnderlyingType",
    "autocorr",
    "parcorr",
    "crosscorr",
    "lbqtest",
    "archtest",
    "adftest",
    "kpsstest",
    "pptest",
    "lagmatrix",
    "arima",
    "estimate",
    "forecast",
    "infer",
    "simulate",
    "summarize",
    "aicbic",
    "selforgmap",
    "gridtop",
    "hextop",
    "randtop",
    "dist",
    "linkdist",
    "boxdist",
    "mandist",
    "narxnet",
    "narnet",
    "timedelaynet",
    "preparets",
    "closeloop",
    "openloop",
    "removedelay",
    "tonndata",
    "fromnndata",
    "con2seq",
    "seq2con",
    "plotsomtop",
    "plotsomnc",
    "plotsomnd",
    "plotsomplanes",
    "plotsomhits",
    "plotsompos",
    "train",
    "dividerand",
    "divideblock",
    "divideint",
    "divideind",
    "dividetrain",
    "cascadeforwardnet",
    "configure",
    "crossentropy",
    "feedforwardnet",
    "fitnet",
    "getwb",
    "hardlim",
    "ind2vec",
    "init",
    "isconfigured",
    "logsig",
    "mae",
    "mapminmax",
    "mapstd",
    "mse",
    "patternnet",
    "perform",
    "poslin",
    "purelin",
    "radbas",
    "removeconstantrows",
    "sae",
    "satlin",
    "setwb",
    "softmax",
    "sse",
    "tansig",
    "vec2ind",
    "uigetfile",
    "uiputfile",
    "uigetdir",
    "inputdlg",
    "questdlg",
    "msgbox",
    "errordlg",
    "warndlg",
    "helpdlg",
    "waitbar",
    "uiwait",
    "waitfor",
    "listdlg",
    "ttest",
    "ttest2",
    "anova1",
    "anova2",
    "kstest",
    "chi2gof",
    "ranksum",
    "signrank",
    "trimmean",
    "randsample",
    "datasample",
    "bootstrp",
    "lassoPlot",
    "regress",
    "ridge",
    "lasso",
    "pca",
    "pcacov",
    "kmeans",
    "kmedoids",
    "pdist",
    "pdist2",
    "squareform",
    "linkage",
    "dendrogram",
    "makedist",
    "fitdist",
    "designfilt",
    "fitcknn",
    "fitctree",
    "fitcsvm",
    "fitcdiscr",
    "fitclinear",
    "fitcecoc",
    "fitcensemble",
    "templateSVM",
    "templateTree",
    "templateLinear",
    "templateDiscriminant",
    "cvpartition",
    "training",
    "test",
    "repartition",
    "loss",
    "oobLoss",
    "predictorImportance",
    "fitcnb",
    "fitrtree",
    "fitrsvm",
    "fitrgp",
    "fitrensemble",
    "fitrlinear",
    "prune",
    "crossval",
    "predict",
    "resubLoss",
    "kfoldLoss",
    "kfoldPredict",
    "fitlm",
    "fitglm",
    "stepwiselm",
    "fit",
    "fittype",
    "fitoptions",
    "cfit",
    "sfit",
    "confint",
    "predint",
    "coeffnames",
    "coeffvalues",
    "numcoeffs",
    "numargs",
    "dependnames",
    "indepnames",
    "probnames",
    "probvalues",
    "islinear",
    "category",
    "differentiate",
    "integrate",
    "cluster",
    "inconsistent",
    "knnsearch",
    "confusionmat",
    "periodogram",
    "pwelch",
    "cpsd",
    "mscohere",
    "tfestimate",
    "spectrogram",
    "stft",
    "istft",
    "findpeaks",
    "resample",
    "decimate",
    "interp",
    "hilbert",
    "envelope",
    "medfilt1",
    "sgolayfilt",
    "sgolay",
    "lowpass",
    "highpass",
    "bandpass",
    "bandstop",
    "fir2",
    "firls",
    "firpm",
    "ctrb",
    "obsv",
    "care",
    "dare",
    "lqr",
    "dlqr",
    "place",
    "acker",
    "rlocus",
    "step",
    "impulse",
    "initial",
    "lsim",
    "stepinfo",
    "bode",
    "nyquist",
    "freqresp",
    "evalfr",
    "margin",
    "allmargin",
    "nichols",
    "ngrid",
    "sigma",
    "lyap",
    "dlyap",
    "pzmap",
    "zplane",
    "pid",
    "pidstd",
    "pidtune",
    "pidtuneOptions",
    "string",
    "strings",
    "missing",
    "ismissing",
    "isstring",
    "strlength",
    "strip",
    "contains",
    "count",
    "startsWith",
    "endsWith",
    "replace",
    "split",
    "join",
    "compose",
    "extractBefore",
    "extractAfter",
    "extractBetween",
    "insertBefore",
    "insertAfter",
    "erase",
    "reverse",
    "pad",
    "newline",
    "butter",
    "cheby2",
    "ellip",
    "besself",
    "buttord",
    "cheb1ord",
    "cheb2ord",
    "iirpeak",
    "iirnotch",
    "firpmord",
    "kaiserord",
    "ellipord",
    "zp2tf",
    "tf2zp",
    "zp2sos",
    "sos2zp",
    "tf2sos",
    "sos2tf",
    "zp2ss",
    "ss2zp",
    "tf2ss",
    "ss2tf",
    "bilinear",
    "isMATLABReleaseOlderThan",
    "path",
    "addpath",
    "rmpath",
    "genpath",
    "restoredefaultpath",
    "prefdir",
    "userpath",
    "savepath",
    "garch",
    "egarch",
    "gjr",
    "int2bit",
    "bit2int",
    "de2bi",
    "bi2de",
    "pskmod",
    "pskdemod",
    "qammod",
    "qamdemod",
    "pammod",
    "pamdemod",
    "genqammod",
    "genqamdemod",
    "awgn",
    "wgn",
    "biterr",
    "symerr",
    "berawgn",
    "qfunc",
    "qfuncinv",
    "rcosdesign",
    "fskmod",
    "fskdemod",
    "dpskmod",
    "dpskdemod",
    "mskmod",
    "mskdemod",
    "ofdmmod",
    "ofdmdemod",
    "scatterplot",
    "eyediagram",
    "berfit",
    "trimf",
    "trapmf",
    "gaussmf",
    "gauss2mf",
    "gbellmf",
    "sigmf",
    "dsigmf",
    "psigmf",
    "zmf",
    "smf",
    "pimf",
    "linsmf",
    "linzmf",
    "evalmf",
    "defuzz",
    "mamfis",
    "sugfis",
    "fisvar",
    "fismf",
    "fisrule",
    "addInput",
    "addOutput",
    "addMF",
    "removeInput",
    "removeOutput",
    "removeMF",
    "addRule",
    "showrule",
    "evalfis",
    "evalfisOptions",
    "readfis",
    "writefis",
    "gensurf",
    "gensurfOptions",
    "plotmf",
    "plotfis",
    "beep",
    "input",
    "system",
    "exit",
    "quit",
    "display",
    "isfile",
    "isfolder",
    "im2double",
    "im2single",
    "im2uint8",
    "im2uint16",
    "mat2gray",
    "imcomplement",
    "rgb2gray",
    "ind2rgb",
    "imresize",
    "imrotate",
    "imcrop",
    "imtranslate",
    "imref2d",
    "affinetform2d",
    "rigidtform2d",
    "simtform2d",
    "transltform2d",
    "affine2d",
    "transformPointsForward",
    "transformPointsInverse",
    "invert",
    "outputLimits",
    "isRigid",
    "isSimilarity",
    "isTranslation",
    "worldToIntrinsic",
    "intrinsicToWorld",
    "worldToSubscript",
    "sizesMatch",
    "cameraIntrinsics",
    "cameraParameters",
    "generateCheckerboardPoints",
    "worldToImage",
    "rotationVectorToMatrix",
    "rotationMatrixToVector",
    "rotvec2mat3d",
    "rotmat2vec3d",
    "undistortImage",
    "undistortPoints",
    "imwarp",
    "imregtform",
    "imregister",
    "fspecial",
    "imfilter",
    "imgaussfilt",
    "medfilt2",
    "imregconfig",
    "imfuse",
    "imshowpair",
    "detectCheckerboardPoints",
    "estimateCameraParameters",
    "showReprojectionErrors",
    "showExtrinsics",
    "displayErrors",
    "csaps",
    "fnval",
    "csapi",
    "csape",
    "spaps",
    "spap2",
    "spmak",
    "fnbrk",
    "aptknt",
    "augknt",
    "fnder",
    "fnint",
    "fnplt",
    "profile",
    "profsave",
    "categorical",
    "categories",
    "countcats",
    "iscategorical",
    "isundefined",
    "timetable",
    "table2timetable",
    "timetable2table",
    "istimetable",
    "retime",
    "synchronize",
    "summary",
    "calendarDuration",
    "calyears",
    "calmonths",
    "calweeks",
    "caldays",
    "iscalendarduration",
    "between",
    "datetime",
    "duration",
    "NaT",
    "seconds",
    "minutes",
    "hours",
    "days",
    "years",
    "milliseconds",
    "isnat",
    "isdatetime",
    "isduration",
    "posixtime",
    "juliandate",
    "exceltime",
    "ymd",
    "hms",
    "timeofday",
    "year",
    "month",
    "day",
    "hour",
    "minute",
    "second",
    "parfor",
    "spmd",
    "parpool",
    "gcp",
    "parfeval",
    "batch",
    "fetchOutputs",
    "fetchNext",
    "gpuArray",
    "gather",
    "spmdIndex",
    "spmdSize",
    "labindex",
    "numlabs",
    "varargin",
    "linkaxes",
    "box",
    "gscatter",
    "xticks",
    "yticks",
    "zticks",
    "xticklabels",
    "yticklabels",
    "zticklabels",
    "xtickangle",
    "ytickangle",
    "ztickangle",
    "xtickformat",
    "ytickformat",
    "ztickformat",
    "colororder",
    "plot3",
    "isosurface",
    "isonormals",
    "isocaps",
    "scatter3",
    "view",
    "surf",
    "mesh",
    "shading",
    "zlim",
    "gausswin",
    "tukeywin",
    "chebwin",
    "taylorwin",
    "barthannwin",
    "bohmanwin",
    "parzenwin",
    "ishghandle",
    "findobj",
    "allchild",
    "groot",
    "reset",
    "xline",
    "yline",
    "rectangle",
    "lsqminnorm",
    "givens",
    "makima",
    "griddedInterpolant",
    "scatteredInterpolant",
    "vecnorm",
    "odeplot",
    "odeprint",
    "pdepe",
    "pdeval",
    "phasedelay",
    "subtitle",
    "zlabel",
    "xcov",
    "optimplotx",
    "optimplotfval",
    "normpdf",
    "normcdf",
    "norminv",
    "normrnd",
    "unifpdf",
    "unifcdf",
    "unifinv",
    "unifrnd",
    "exppdf",
    "expcdf",
    "expinv",
    "exprnd",
    "gampdf",
    "gamcdf",
    "gaminv",
    "gamrnd",
    "betapdf",
    "betacdf",
    "betainv",
    "betarnd",
    "chi2pdf",
    "chi2cdf",
    "chi2inv",
    "chi2rnd",
    "tpdf",
    "tcdf",
    "tinv",
    "trnd",
    "fpdf",
    "fcdf",
    "finv",
    "frnd",
    "lognpdf",
    "logncdf",
    "logninv",
    "lognrnd",
    "wblpdf",
    "wblcdf",
    "wblinv",
    "wblrnd",
    "raylpdf",
    "raylcdf",
    "raylinv",
    "raylrnd",
    "evpdf",
    "evcdf",
    "evinv",
    "evrnd",
    "binopdf",
    "binocdf",
    "binoinv",
    "binornd",
    "poisspdf",
    "poisscdf",
    "poissinv",
    "poissrnd",
    "geopdf",
    "geocdf",
    "geoinv",
    "geornd",
    "nbinpdf",
    "nbincdf",
    "nbininv",
    "nbinrnd",
    "pdf",
    "cdf",
    "icdf",
    "random",
    "normfit",
    "expfit",
    "paramci",
    "negloglik",
    "mle",
    "statset",
    "normlike",
    "explike",
    "gamlike",
    "wbllike",
    "lognlike",
    "evlike",
    "betalike",
    "gamfit",
    "wblfit",
    "lognfit",
    "betafit",
    "raylfit",
    "evfit",
    "poissfit",
    "binofit",
    "nbinfit",
    "gevfit",
    "gpfit",
    "probplot",
    "normplot",
    "wblplot",
    "qqplot",
    "ecdf",
    "cdfplot",
    "wfilters",
    "dwt",
    "idwt",
    "dwtmode",
    "wavedec",
    "waverec",
    "appcoef",
    "detcoef",
    "wrcoef",
    "wmaxlev",
    "wthresh",
    "thselect",
    "wden",
    "wdenoise",
    "modwt",
    "imodwt",
    "modwtmra",
    "emd",
    "vmd",
    "ewt",
    "cwt",
    "icwt",
    "cwtfilterbank",
    "wsst",
    "iwsst",
    "optimoptions",
    "linprog",
    "quadprog",
    "mpc",
    "getEstimator",
    "setEstimator",
    "getindist",
    "setindist",
    "getoutdist",
    "setoutdist",
    "mpcstate",
    "mpcmove",
    "mpcmoveopt",
    "mpcsimopt",
    "review",
    "setmpcsignals",
    "mpcActiveSetSolver",
    "mpcActiveSetOptions",
    "lsqlin",
    "fmincon",
    "intlinprog",
    "fminimax",
    "fgoalattain",
    "patternsearch",
    "ga",
    "particleswarm",
    "simulannealbnd",
    "fminunc",
    "lsqnonlin",
    "lsqcurvefit",
    "fsolve",
    "dicomlookup",
    "dicomdict",
    "dicominfo",
    "dicomCollection",
    "dicomread",
    "dicomwrite",
    "VideoWriter",
    "open",
    "writeVideo",
    "VideoReader",
    "hasFrame",
    "readFrame",
    "read",
    "methods",
    "isprop",
    "classdef",
    "isvalid",
    "superclasses",
    "handle",
    "events",
    "addlistener",
    "notify",
    "enumeration",
    "metaclass",
    "import",
    "slice",
    "contourslice",
    "volshow",
    "sliceViewer",
    "orthosliceViewer",
    "getAxesHandles",
    "alphamap",
    "flow",
    "smooth3",
    "reducevolume",
    "subvolume",
    "volumebounds",
    "matlab.lang.makeValidName",
    "tf",
    "zpk",
    "ss",
    "tfdata",
    "zpkdata",
    "ssdata",
    "feedback",
    "series",
    "parallel",
    "append",
    "pole",
    "zero",
    "dcgain",
    "minreal",
    "c2d",
    "d2c",
    "isstable",
    "issiso",
    "order",
    "damp",
    "pade",
    "gram",
    "hsvd",
    "balreal",
    "balred",
    "balredOptions",
    "modred",
    "filtord",
    "prctile",
    "boxplot",
    "histfit",
    "confusion",
    "plotconfusion",
    "ploterrhist",
    "plotperform",
    "plotregression",
    "plotroc",
    "plottrainstate",
    "roc",
    "perfcurve",
    "gplot",
    "quantile",
    "zscore",
    "skewness",
    "kurtosis",
    "mad",
    "normalize",
    "rescale",
    "smoothdata",
    "isoutlier",
    "filloutliers",
    "rmoutliers",
    "bounds",
    "rms",
    "text",
    "colormap",
    "image",
    "imagesc",
    "imshow",
    "colorbar",
    "clim",
    "caxis",
    "yyaxis",
    "drawnow",
    "ishandle",
    "viridis",
    "parula",
    "gray",
    "hot",
    "cool",
    "jet",
    "hsv",
    "bone",
    "copper",
    "histogram",
    "errorbar",
    "area",
    "barh",
    "pie",
    "fill",
    "polarplot",
    "fplot",
    "fsurf",
    "fimplicit",
    "line",
    "quiver",
    "quiver3",
    "contour3",
    "animatedline",
    "addpoints",
    "getpoints",
    "clearpoints",
    "print",
    "heatmap",
    "contourc",
    "contour",
    "contourf",
    "clabel",
    "jsondecode",
    "jsonencode",
    "readcell",
    "readtable",
    "writetable",
    "detectImportOptions",
    "setvartype",
    "getvaropts",
    "setvaropts",
    "preview",
    "readtimetable",
    "table",
    "array2table",
    "cell2table",
    "struct2table",
    "table2array",
    "table2cell",
    "table2struct",
    "istable",
    "height",
    "width",
    "head",
    "tail",
    "addvars",
    "removevars",
    "renamevars",
    "movevars",
    "innerjoin",
    "outerjoin",
    "findgroups",
    "splitapply",
    "groupsummary",
    "varfun",
    "rowfun",
    "rmmissing",
    "fillmissing",
    "standardizeMissing",
    "writecell",
    "xlsread",
    "xlswrite",
    "webread",
    "websave",
    "weboptions",
    "h5read",
    "h5create",
    "h5write",
    "ncread",
    "ichol",
    "ilu",
    "sparse",
    "issparse",
    "speye",
    "spalloc",
    "spdiags",
    "spones",
    "sprand",
    "sprandn",
    "sprandsym",
    "spfun",
    "amd",
    "colamd",
    "symamd",
    "symrcm",
    "nzmax",
    "imhist",
    "stretchlim",
    "imadjust",
    "graythresh",
    "imbinarize",
    "rgb2lab",
    "lab2rgb",
    "rgb2ycbcr",
    "ycbcr2rgb",
    "multithresh",
    "imquantize",
    "adaptthresh",
    "label2rgb",
    "labeloverlay",
    "imoverlay",
    "boundarymask",
    "impixel",
    "otsuthresh",
    "strel",
    "imdilate",
    "imerode",
    "imopen",
    "imclose",
    "imtophat",
    "imbothat",
    "bwmorph",
    "imfill",
    "bwareaopen",
    "imclearborder",
    "bwlabel",
    "bwdist",
    "watershed",
    "activecontour",
    "imsegkmeans",
    "grayconnected",
    "bwboundaries",
    "bwperim",
    "bwarea",
    "bwareafilt",
    "bwpropfilt",
    "bwselect",
    "bwconncomp",
    "labelmatrix",
    "regionprops",
    "histeq",
    "edge",
    "iddata",
    "idpoly",
    "idtf",
    "idss",
    "idproc",
    "arx",
    "armax",
    "oe",
    "bj",
    "tfest",
    "ssest",
    "n4sid",
    "procest",
    "compare",
    "resid",
    "sim",
    "readlines",
    "writelines",
    "ferror",
    "dos",
    "unix",
    "edit",
    "web",
    // PDE
    "decsg",
    "csgdel",
    "createpde",
    "geometryFromEdges",
    "pdegplot",
    "generateMesh",
    "findNodes",
    "findElements",
    "pdemesh",
    "pdeplot",
    "specifyCoefficients",
    "applyBoundaryCondition",
    "setInitialConditions",
    "solvepde",
    "solvepdeeig",
    "interpolateSolution",
    "evaluateGradient",
    // ui* components and App Designer
    "uiresume",
    "uifigure",
    "uigridlayout",
    "uipanel",
    "uitabgroup",
    "uitab",
    "uibutton",
    "uilabel",
    "uieditfield",
    "uitextarea",
    "uidropdown",
    "uilistbox",
    "uicheckbox",
    "uibuttongroup",
    "uiradiobutton",
    "uislider",
    "uispinner",
    "uiswitch",
    "uitable",
    "uiaxes",
    "uialert",
    "uiconfirm",
    "uiprogressdlg",
    "uicontrol",
    "gcbo",
    "gcbf",
    "createCallbackFcn",
    "registerApp",
    "runStartupFcn",
    "guidata",
    "mrUiFire",
    "cla",
    "findall",
    // audio
    "sound",
    "soundsc",
    "audioplayer",
    "audiorecorder",
    "play",
    "playblocking",
    "resume",
    "stop",
    "isplaying",
    "record",
    "recordblocking",
    "getaudiodata",
    "getplayer",
    // geometry, graphics, stats, linalg, GPU stubs
    "bucky",
    "canUseGPU",
    "convexHull",
    "decomposition",
    "delaunayTriangulation",
    "edges",
    "eqtflength",
    "gpuDevice",
    "gpuDeviceCount",
    "ishold",
    "mnrnd",
    "movegui",
    "nearestNeighbor",
    "neighbors",
    "pcolor",
    "pointLocation",
    "sphere",
    "surface",
    "surfl",
    "tetramesh",
    "trimesh",
    "trisurf",
    "vertexAttachments",
    "voronoi",
];

/// Command-line app launchers added since v0.1.2.
const APP_LAUNCHERS_SINCE_V012: &[&str] = &[
    "nnstart",
    "nftool",
    "nprtool",
    "nctool",
    "ntstool",
    "bertool",
    "signalAnalyzer",
    "fvtool",
    "windowDesigner",
    "signalMultiresolutionAnalyzer",
    "waveletSignalDenoiser",
    "waveletTimeFrequencyAnalyzer",
    "pidTuner",
    "controlSystemDesigner",
    "linearSystemAnalyzer",
    "modelReducer",
    "systemIdentification",
    "curveFitter",
    "cftool",
    "splineTool",
    "distributionFitter",
    "disttool",
    "randtool",
    "classificationLearner",
    "regressionLearner",
    "imageViewer",
    "imtool",
    "imageSegmenter",
    "colorThresholder",
    "imageRegionAnalyzer",
    "imageBatchProcessor",
    "dataCleaner",
    "uiimport",
    "wirelessWaveformGenerator",
    "mpcDesigner",
    "econometricModeler",
    "fuzzyLogicDesigner",
    "implay",
    "visdiff",
    "dicomBrowser",
    "registrationEstimator",
    "cameraCalibrator",
    "volumeViewer",
    "pdeModeler",
    "appdesigner",
];

/// Property-bearing object types added since v0.1.2.
const OBJECT_TYPES_SINCE_V012: &[&str] = &[
    "VideoReader",
    "VideoWriter",
    "network",
    "digitalFilter",
    "cwtfilterbank",
    "arima",
    "garch",
    "egarch",
    "gjr",
    "tf",
    "zpk",
    "ss",
    "mpc",
    "mpcstate",
    "iddata",
    "idpoly",
    "idtf",
    "idss",
    "idproc",
    "mamfis",
    "sugfis",
    "fisvar",
    "fismf",
    "fisrule",
    "griddedInterpolant",
    "scatteredInterpolant",
    "fittype",
    "cfit",
    "sfit",
    "fitoptions",
    "cvpartition",
    "LinearModel",
    "GeneralizedLinearModel",
    "ClassificationKNN",
    "ClassificationTree",
    "ClassificationSVM",
    "ClassificationNaiveBayes",
    "ClassificationDiscriminant",
    "ClassificationLinear",
    "ClassificationECOC",
    "ClassificationEnsemble",
    "ClassificationBaggedEnsemble",
    "ClassificationPartitionedModel",
    "RegressionTree",
    "RegressionSVM",
    "RegressionGP",
    "RegressionEnsemble",
    "RegressionLinear",
    "RegressionPartitionedModel",
    "prob.NormalDistribution",
    "prob.UniformDistribution",
    "prob.ExponentialDistribution",
    "prob.GammaDistribution",
    "prob.BetaDistribution",
    "prob.ChiSquareDistribution",
    "prob.TDistribution",
    "prob.FDistribution",
    "prob.LognormalDistribution",
    "prob.WeibullDistribution",
    "prob.RayleighDistribution",
    "prob.ExtremeValueDistribution",
    "prob.BinomialDistribution",
    "prob.PoissonDistribution",
    "prob.GeometricDistribution",
    "prob.NegativeBinomialDistribution",
    "prob.LogisticDistribution",
    "prob.LoglogisticDistribution",
    "prob.RicianDistribution",
    "prob.NakagamiDistribution",
    "prob.GeneralizedExtremeValueDistribution",
    "prob.GeneralizedParetoDistribution",
    "prob.tLocationScaleDistribution",
    "prob.InverseGaussianDistribution",
    "prob.BirnbaumSaundersDistribution",
    "prob.HalfNormalDistribution",
    "prob.KernelDistribution",
    "parallel.ThreadPool",
    "parallel.ProcessPool",
    "parallel.FevalFuture",
    "parallel.job.CJSIndependentJob",
    "table",
    "timetable",
    "datetime",
    "duration",
    "categorical",
    "matlab.io.text.DelimitedTextImportOptions",
    "matlab.io.spreadsheet.SpreadsheetImportOptions",
    "matlab.io.NumericVariableImportOptions",
    "matlab.io.TextVariableImportOptions",
    "matlab.io.DatetimeVariableImportOptions",
    "matlab.io.DurationVariableImportOptions",
    "matlab.io.CategoricalVariableImportOptions",
    "matlab.io.LogicalVariableImportOptions",
    "strel",
    "imref2d",
    "affinetform2d",
    "rigidtform2d",
    "simtform2d",
    "transltform2d",
    "affine2d",
    "registration.optimizer.RegularStepGradientDescent",
    "registration.optimizer.OnePlusOneEvolutionary",
    "registration.metric.MeanSquares",
    "registration.metric.MattesMutualInformation",
    "cameraIntrinsics",
    "cameraParameters",
    "cameraCalibrationErrors",
    "intrinsicsEstimationErrors",
    "extrinsicsEstimationErrors",
    "Root",
    "Annotation",
    "Text",
    "ColorBar",
    "Image",
    "Volume",
    "SliceViewer",
    "OrthosliceViewer",
    "TiledChartLayout",
    "NumericRuler",
    "ConstantLine",
    "Rectangle",
    "Patch",
    // PDE
    "pde.PDEModel",
    "pde.AnalyticGeometry",
    "pde.FEMesh",
    "pde.StationaryResults",
    "pde.TimeDependentResults",
    "pde.EigenResults",
    // ui* components and App Designer
    "matlab.apps.AppBase",
    "Figure",
    "GridLayout",
    "Panel",
    "TabGroup",
    "Tab",
    "Button",
    "StateButton",
    "Label",
    "EditField",
    "NumericEditField",
    "TextArea",
    "DropDown",
    "ListBox",
    "CheckBox",
    "ButtonGroup",
    "RadioButton",
    "Slider",
    "Spinner",
    "Switch",
    "Table",
    "UIAxes",
    "ProgressDialog",
    "UIControl",
    // audio objects
    "audioplayer",
    "audiorecorder",
    // triangulation and decomposition
    "delaunayTriangulation",
    "decomposition",
];

/// Qualified `Type.method` / `Type.Property` names added since v0.1.2.
const OBJECT_MEMBERS_SINCE_V012: &[&str] = &[
    "VideoReader.hasFrame",
    "VideoReader.readFrame",
    "VideoReader.read",
    "VideoWriter.open",
    "VideoWriter.writeVideo",
    "VideoWriter.close",
    "imref2d.contains",
    "imref2d.intrinsicToWorld",
    "imref2d.sizesMatch",
    "imref2d.worldToIntrinsic",
    "imref2d.worldToSubscript",
    "affinetform2d.transformPointsForward",
    "affinetform2d.transformPointsInverse",
    "affinetform2d.invert",
    "affinetform2d.outputLimits",
    "affinetform2d.isRigid",
    "affinetform2d.isSimilarity",
    "affinetform2d.isTranslation",
    "cameraIntrinsics.worldToImage",
    "cameraParameters.worldToImage",
    "cameraCalibrationErrors.displayErrors",
    "cwtfilterbank.wt",
    "cwtfilterbank.freqz",
    "cwtfilterbank.wavelets",
    "cwtfilterbank.scales",
    "cwtfilterbank.centerFrequencies",
    "cwtfilterbank.centerPeriods",
    "LinearModel.predict",
    "GeneralizedLinearModel.predict",
    "ClassificationKNN.predict",
    "ClassificationTree.predict",
    "RegressionTree.predict",
    "cvpartition.training",
    "cvpartition.test",
    "cvpartition.repartition",
    "arima.estimate",
    "arima.forecast",
    "arima.infer",
    "arima.simulate",
    "arima.summarize",
    "garch.estimate",
    "garch.forecast",
    "garch.infer",
    "garch.simulate",
    "garch.summarize",
    "cfit.confint",
    "cfit.predint",
    "cfit.differentiate",
    "cfit.integrate",
    "cfit.coeffvalues",
    "cfit.coeffnames",
    "parallel.FevalFuture.fetchOutputs",
    "parallel.FevalFuture.wait",
    "parallel.FevalFuture.cancel",
    "mpc.mpcmove",
    "mpc.sim",
    // PDE
    "pde.PDEModel.geometryFromEdges",
    "pde.PDEModel.pdegplot",
    "pde.PDEModel.generateMesh",
    "pde.PDEModel.pdemesh",
    "pde.PDEModel.pdeplot",
    "pde.PDEModel.specifyCoefficients",
    "pde.PDEModel.applyBoundaryCondition",
    "pde.PDEModel.setInitialConditions",
    "pde.PDEModel.solvepde",
    "pde.PDEModel.solvepdeeig",
    "pde.AnalyticGeometry.pdegplot",
    "pde.FEMesh.findNodes",
    "pde.FEMesh.findElements",
    "pde.FEMesh.pdemesh",
    "pde.FEMesh.pdeplot",
    "pde.StationaryResults.interpolateSolution",
    "pde.StationaryResults.evaluateGradient",
    "pde.TimeDependentResults.interpolateSolution",
    "pde.TimeDependentResults.evaluateGradient",
    "pde.EigenResults.interpolateSolution",
    // ui* components and App Designer
    "matlab.apps.AppBase.registerApp",
    "matlab.apps.AppBase.runStartupFcn",
    "matlab.apps.AppBase.createCallbackFcn",
    // audio objects
    "audioplayer.play",
    "audioplayer.playblocking",
    "audioplayer.pause",
    "audioplayer.resume",
    "audioplayer.stop",
    "audioplayer.isplaying",
    "audiorecorder.record",
    "audiorecorder.recordblocking",
    "audiorecorder.stop",
    "audiorecorder.getaudiodata",
    "audiorecorder.getplayer",
    // triangulation
    "delaunayTriangulation.pointLocation",
    "delaunayTriangulation.nearestNeighbor",
    "delaunayTriangulation.edges",
    "delaunayTriangulation.neighbors",
    "delaunayTriangulation.convexHull",
    "delaunayTriangulation.vertexAttachments",
];

#[test]
fn every_name_added_since_v012_has_a_page() {
    let path = PathContext::default();
    let groups: [(&str, &[&str]); 4] = [
        ("function", FUNCTIONS_SINCE_V012),
        ("app launcher", APP_LAUNCHERS_SINCE_V012),
        ("object type", OBJECT_TYPES_SINCE_V012),
        ("object member", OBJECT_MEMBERS_SINCE_V012),
    ];
    let mut missing = Vec::new();
    for (kind, names) in groups {
        for &name in names {
            let Some(page) = help::page(name) else {
                missing.push(format!("{kind} {name}: no page"));
                continue;
            };
            if page.trim().is_empty() {
                missing.push(format!("{kind} {name}: empty page"));
                continue;
            }
            match help::help(name, &path) {
                Ok(text)
                    if text == page
                        && !text.starts_with(&format!("\n{name} not found."))
                        && text != format!("{name} is supported. No help page yet.")
                        && text
                            != format!(
                                "{name} is a MATLAB function that MatRust does not support yet."
                            ) => {}
                Ok(text) => missing.push(format!("{kind} {name}: unexpected help text: {text}")),
                Err(err) => missing.push(format!("{kind} {name}: help error: {err}")),
            }
        }
    }
    // Every `Type.method` page must also resolve in `Type/method` slash form.
    for &name in OBJECT_MEMBERS_SINCE_V012 {
        let (class, member) = name.rsplit_once('.').expect("qualified member");
        let slash = format!("{class}/{member}");
        let dot_text = help::help(name, &path).unwrap();
        let slash_text = help::help(&slash, &path)
            .unwrap_or_else(|e| panic!("slash help failed for {slash}: {e}"));
        assert_eq!(slash_text, dot_text, "{slash}");
    }
    assert!(
        missing.is_empty(),
        "missing or unresolved help pages:\n{}",
        missing.join("\n")
    );
    // Spot-check through the full Engine `help <name>` command path.
    for command in [
        "help VideoReader.readFrame",
        "help VideoReader/readFrame",
        "help pde.PDEModel/generateMesh",
        "help prob.NormalDistribution/pdf",
        "help tf/step",
        "help Figure/clf",
        "help cwtfilterbank.wt",
        "help cwtfilterbank/wt",
        "help prob.NormalDistribution",
        "help LinearModel",
        "help volumeViewer",
    ] {
        let text = help(command);
        assert!(
            !text.trim().is_empty() && !text.contains(" not found."),
            "{command}: {text}"
        );
    }
}

#[test]
fn user_class_method_file_beats_a_builtin_member_page() {
    let directory = tempfile::tempdir().unwrap();
    let class_dir = directory.path().join("@cwtfilterbank");
    std::fs::create_dir(&class_dir).unwrap();
    std::fs::write(
        class_dir.join("wt.m"),
        "function y = wt(fb, x)\n% WT  User override of the transform.\ny = x;\nend\n",
    )
    .unwrap();
    let path = PathContext {
        directory: Some(directory.path().to_path_buf()),
        path: Vec::new(),
    };
    for name in ["cwtfilterbank.wt", "cwtfilterbank/wt", "@cwtfilterbank/wt"] {
        let text = help::help(name, &path).unwrap();
        assert!(
            text.contains("User override of the transform."),
            "{name}: {text}"
        );
    }
    let builtin = help::help("cwtfilterbank.wt", &PathContext::default()).unwrap();
    assert!(
        builtin.contains("Syntax: [cfs, f] = wt(fb, x)"),
        "{builtin}"
    );
    assert_eq!(builtin, help::page("cwtfilterbank.wt").unwrap());
}

#[test]
fn a_bare_package_class_name_opens_its_page() {
    let full = help("help pde.PDEModel");
    assert!(full.contains("PDEModel"), "{full}");
    assert_eq!(help("help PDEModel"), full);
    let normal = help("help prob.NormalDistribution");
    assert!(
        normal.contains("Normal probability distribution object"),
        "{normal}"
    );
    assert_eq!(help("help NormalDistribution"), normal);
    // Lowercase names keep the not-found text.
    assert!(help("help pdemodel").contains("pdemodel not found."));
}

#[test]
fn package_constructor_precedes_a_user_class_method_on_the_path() {
    let directory = tempfile::tempdir().unwrap();
    let folder = directory.path().join("functions");
    let class = folder.join("@containers");
    fs::create_dir_all(&class).unwrap();
    fs::write(
        class.join("Map.m"),
        "function Map()\n%User Map method.\nend\n",
    )
    .unwrap();
    let mut engine = Engine::default();
    engine.set_current_folder(directory.path()).unwrap();
    output(&mut engine, "addpath('functions')");
    let page = help::page("containers.Map").unwrap();
    assert!(page.contains("Syntax: m = containers.Map(keys, values)"));
    assert_eq!(output(&mut engine, "help containers.Map"), page);
    assert_eq!(output(&mut engine, "doc containers.Map"), page);
    assert_eq!(
        output(&mut engine, "help('@containers/Map')"),
        " User Map method."
    );
    let package = folder.join("+containers");
    fs::create_dir(&package).unwrap();
    fs::write(package.join("Map.m"), "%User package constructor.\n").unwrap();
    for command in ["help containers.Map", "doc containers.Map"] {
        assert_eq!(output(&mut engine, command), " User package constructor.");
    }
}

#[test]
fn package_topics_have_no_class_method_slash_alias() {
    let mut engine = Engine::default();
    for name in [
        "@containers/Map",
        "+matlab/@lang/makeValidName.m",
        "parallel/ThreadPool",
    ] {
        let error = engine.execute(&format!("help('{name}')")).unwrap_err();
        assert!(error.contains("is not a function name"), "{name}: {error}");
    }
}

#[test]
fn app_designer_class_names_open_their_short_name_pages() {
    let path = PathContext::default();
    for (namespace, classes) in [
        ("matlab.ui", &["Figure"][..]),
        (
            "matlab.ui.container",
            &["GridLayout", "Panel", "TabGroup", "Tab", "ButtonGroup"],
        ),
        (
            "matlab.ui.control",
            &[
                "Button",
                "StateButton",
                "Label",
                "EditField",
                "NumericEditField",
                "TextArea",
                "DropDown",
                "ListBox",
                "CheckBox",
                "RadioButton",
                "Slider",
                "Spinner",
                "Switch",
                "Table",
                "UIAxes",
            ],
        ),
    ] {
        for class in classes {
            let name = format!("{namespace}.{class}");
            let page = help::page(class).unwrap();
            assert!(page.contains("Syntax:"), "{class}");
            assert_eq!(help::help(&name, &path).unwrap(), page, "{name}");
            assert_eq!(help::doc(&name, &path).unwrap(), page, "{name}");
        }
    }
    for name in [
        "matlab.ui.container.Button",
        "matlab.ui.control.Panel",
        "matlab.ui.control.button",
    ] {
        assert!(help::help(name, &path).is_err(), "{name}");
    }
}

mod relhelp_h2 {
    use std::process::Command;

    fn check_examples(name: &str, expected: &[&str]) {
        let page = super::help(&format!("help {name}"));
        let examples: Vec<_> = page
            .lines()
            .filter_map(|l| l.strip_prefix("Example: "))
            .collect();
        assert_eq!(examples.len(), expected.len(), "{name}: {page}");
        for (example, &value) in examples.into_iter().zip(expected) {
            let (_, comment) = example
                .rsplit_once("   % ")
                .expect("example needs an output");
            assert_eq!(comment.trim_matches('\''), value, "{name}: {example}");
            // The desktop bin test includes this file; Cargo sets CARGO_BIN_EXE_* only for integration tests.
            let Some(exe) = option_env!("CARGO_BIN_EXE_matrust") else {
                continue;
            };
            let run = Command::new(exe)
                .args(["-batch", example])
                .env("MATRUST_AUDIO_DEVICE", "null")
                .output()
                .unwrap();
            assert!(
                run.status.success(),
                "{name}: {example}\n{}",
                String::from_utf8_lossy(&run.stderr)
            );
            let stdout = String::from_utf8(run.stdout).unwrap();
            let values: Vec<_> = stdout
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && *l != "ans =" && !l.starts_with("Warning: "))
                .collect();
            assert_eq!(values, [value], "{name}: {stdout}");
        }
    }

    #[test]
    fn classification_partitioned_model() {
        check_examples(
            "ClassificationPartitionedModel",
            &["ClassificationPartitionedModel"],
        );
    }

    #[test]
    fn regression_partitioned_model() {
        check_examples(
            "RegressionPartitionedModel",
            &["RegressionPartitionedModel"],
        );
    }

    #[test]
    fn binomial_distribution() {
        check_examples(
            "prob.BinomialDistribution",
            &["prob.BinomialDistribution", "0.5000"],
        );
    }

    #[test]
    fn classification_tree_predict() {
        check_examples("ClassificationTree.predict", &["2"]);
    }

    #[test]
    fn regression_tree_predict() {
        check_examples("RegressionTree.predict", &["20"]);
    }

    #[test]
    fn time_dependent_interpolate_solution() {
        check_examples("pde.TimeDependentResults.interpolateSolution", &["1"]);
    }

    #[test]
    fn time_dependent_evaluate_gradient() {
        check_examples("pde.TimeDependentResults.evaluateGradient", &["1"]);
    }

    #[test]
    fn eigen_interpolate_solution() {
        check_examples("pde.EigenResults.interpolateSolution", &["2"]);
    }

    #[test]
    fn specify_coefficients_for_face() {
        check_examples("pde.PDEModel.specifyCoefficients", &["1"]);
    }

    #[test]
    fn apply_mixed_boundary_condition() {
        check_examples("pde.PDEModel.applyBoundaryCondition", &["1"]);
    }

    #[test]
    fn plot_mesh_node_data() {
        check_examples("pde.FEMesh.pdeplot", &["patch"]);
    }

    #[cfg(feature = "audio")]
    #[test]
    fn resume_paused_audio() {
        check_examples("audioplayer.resume", &["1"]);
    }

    #[test]
    fn root_screen_size() {
        check_examples("Root", &["4"]);
    }
}

mod relhelp_h3 {
    use super::{Engine, help};

    pub(super) const METHOD_PAGES: &[&str] = &[
        "decomposition.mldivide",
        "datetime.char",
        "datetime.minus",
        "duration.seconds",
        "tf.bode",
        "tf.pzmap",
        "tf.impulse",
        "tf.lsim",
        "tf.series",
        "tf.step",
        "tf.feedback",
        "ss.c2d",
        "table.disp",
        "digitalFilter.filter",
        "digitalFilter.freqz",
        "digitalFilter.filtfilt",
        "prob.NormalDistribution.pdf",
        "prob.NormalDistribution.cdf",
        "prob.NormalDistribution.icdf",
        "prob.NormalDistribution.random",
        "Root.set",
        "Root.get",
        "Text.set",
        "Image.set",
        "TiledChartLayout.nexttile",
        "Rectangle.set",
        "Patch.set",
        "Patch.get",
        "VideoReader.getFileFormats",
        "VideoWriter.getProfiles",
        "Figure.get",
        "Figure.set",
        "Figure.close",
        "Figure.findall",
        "Figure.clf",
        "Figure.movegui",
        "Figure.saveas",
        "Figure.exportgraphics",
        "Figure.print",
        "pde.EigenResults.evaluateGradient",
    ];

    #[test]
    fn qualified_method_pages_run_their_printed_examples() {
        for &name in METHOD_PAGES {
            let mut engine = Engine::default();
            let body = engine
                .execute(&format!("help {name}"))
                .unwrap_or_else(|err| panic!("help {name}: {err}"))
                .output
                .join("\n");
            assert!(!body.trim().is_empty(), "empty page for {name}");
            assert_eq!(
                Some(body.as_str()),
                help::page(name).as_deref(),
                "help {name}"
            );
            let example = body
                .lines()
                .find_map(|line| line.strip_prefix("Example: "))
                .filter(|code| !code.trim().is_empty())
                .unwrap_or_else(|| panic!("missing example for {name}"));
            engine
                .execute(example)
                .unwrap_or_else(|err| panic!("{name}: {example}: {err}"));
        }
    }
}
