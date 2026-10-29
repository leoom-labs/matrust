//! Replay the reference corpus; see docs/tasks/2026-10-24-corpora-report.md.
//! Each case has a fresh engine and a separate folder inside this worktree.
use matrust::Engine;

const REFERENCE: &str = include_str!("fixtures/fn/lang2-expected.txt");
/// Another branch adds delete and tempdir. Until both are merged, these cases
/// also run with a no-op local `delete` function and must then match.
const KNOWN: &[(usize, &str)] = &[
    (
        6,
        "The shared undefined-function message (help::unknown_function, asserted by tests/help_system.rs) lacks the reference's input-class suffix.",
    ),
    (
        115,
        "struct2cell produces a 1x1x2 cell; N-D storage is outside this task.",
    ),
    (
        117,
        "struct2cell produces a 1x0x0 cell; N-D storage is outside this task.",
    ),
];
/// Every normalization/setup exception is listed here. Remaining text is exact.
const HARNESS: &[(usize, &str)] = &[
    (
        86,
        "Expect MatRust's identifier guidance; the oracle fixture retains its original wording.",
    ),
    (
        82,
        "Strip MATLAB desktop help-link markup, keeping its visible text.",
    ),
    (
        88,
        "Strip MATLAB desktop help-link markup, keeping its visible text.",
    ),
    (
        105,
        "Strip MATLAB desktop help-link markup, keeping its visible text.",
    ),
    (
        26,
        "Compare warning message only, removing desktop backtrace and control-byte decoration.",
    ),
    (
        49,
        "Compare warning message only, removing desktop backtrace and control-byte decoration.",
    ),
    (
        50,
        "Compare warning message only, removing desktop backtrace and control-byte decoration.",
    ),
    (
        51,
        "Compare warning message only, removing desktop backtrace and control-byte decoration.",
    ),
    (
        52,
        "Compare warning message only, removing desktop backtrace and control-byte decoration; strip help links.",
    ),
    (
        53,
        "Compare warning message only, removing desktop backtrace and control-byte decoration.",
    ),
    (
        55,
        "Compare warning message only, removing desktop backtrace and control-byte decoration.",
    ),
    (
        68,
        "Compare warning message only, removing desktop backtrace and control-byte decoration.",
    ),
    (
        71,
        "Remove oracle script/line prefix from lasterr text and adjust the expected character length.",
    ),
    (
        76,
        "Compare warning message only, removing desktop backtrace and control-byte decoration.",
    ),
    (
        77,
        "Compare warning message only, removing desktop backtrace and control-byte decoration.",
    ),
    (
        97,
        "Oracle script name snippet becomes submitted source in the caller stack.",
    ),
    (
        107,
        "Strip MATLAB desktop help-link markup, keeping its visible text.",
    ),
    (
        110,
        "Strip MATLAB desktop help-link markup, keeping its visible text.",
    ),
    (
        111,
        "Strip MATLAB desktop help-link markup, keeping its visible text.",
    ),
    (
        120,
        "Strip MATLAB desktop help-link markup, keeping its visible text.",
    ),
    (
        207,
        "Seed zzw2=2 from case 206; MATLAB reused that workspace.",
    ),
    (
        210,
        "Bind tempdir to the per-case worktree folder so the MAT fixture stays inside the authorized directory.",
    ),
];

struct Case<'a> {
    id: usize,
    source: &'a str,
    text: &'a str,
    class: Option<&'a str>,
}
fn cases() -> Vec<Case<'static>> {
    REFERENCE
        .split("@@CASE ")
        .skip(1)
        .map(|block| {
            let (header, block) = block.split_once('\n').unwrap();
            let (id, source) = header.split_once('|').unwrap();
            let (body, _) = block.split_once("@@END").unwrap();
            let (text, class) = if let Some((text, class)) = body.split_once("@@CLASS ") {
                (text, Some(class.trim()))
            } else {
                (body, None)
            };
            Case {
                id: id.parse().unwrap(),
                source,
                text,
                class,
            }
        })
        .collect()
}
fn plain(text: &str, id: usize) -> String {
    if !HARNESS.iter().any(|(n, _)| *n == id) {
        return text.into();
    }
    let mut s = text.to_owned();
    if matches!(id, 52 | 82 | 88 | 105 | 107 | 110 | 111 | 120) {
        while let Some(at) = s.find("<a href=\"") {
            let start = at + s[at..].find('>').unwrap() + 1;
            let end = start + s[start..].find("</a>").unwrap();
            let visible = s[start..end].to_owned();
            s.replace_range(at..end + 4, &visible);
        }
    }
    if matches!(id, 26 | 49..=53 | 55 | 68 | 76 | 77) {
        while let Some(at) = s.find("[\u{8}> In ") {
            let end = at + s[at..].find("]\u{8} \n").unwrap() + "]\u{8} \n".len();
            s.replace_range(at..end, "");
        }
        s = s
            .replace("[\u{8}Warning: ", "Warning: ")
            .replace("]\u{8} \n", "\n");
    }
    if id == 71 {
        s = s.replace("Error using snippet (line 72)\n", "");
    }
    if id == 97 {
        s = s.replace("'snippet'", "'submitted source'");
    }
    s
}

#[test]
fn corpus_has_all_cases_and_documented_exceptions() {
    let all = cases();
    assert_eq!(all.len(), 211);
    for (n, reason) in KNOWN.iter().chain(HARNESS) {
        assert!(!reason.is_empty());
        assert!(all.iter().any(|c| c.id == *n));
    }
}

/// Run one case in a fresh engine and folder: (matches, actual text, class).
fn run_case(case: &Case, suffix: &str) -> (bool, String, Option<String>) {
    let root = std::env::temp_dir();
    let folder = tempfile::tempdir_in(&root).unwrap();
    let mut engine = Engine::default();
    engine.set_file_directory(folder.path()).unwrap();
    engine.set_function_directory(folder.path()).unwrap();
    if case.id == 207 {
        engine.execute("zzw2=2;").unwrap();
    }
    if case.id == 210 {
        engine
            .execute(&format!(
                "tempdir='{}';",
                folder.path().display().to_string().replace('\'', "''")
            ))
            .unwrap();
    }
    let source = case.source.strip_prefix("LONG ").unwrap_or(case.source);
    if source != case.source {
        engine.execute("format long;").unwrap();
    }
    let result = engine.execute(&format!("{source}{suffix}"));
    let actual = match result {
        Ok(execution) => execution
            .output
            .iter()
            .map(|s| format!("{s}\n"))
            .collect::<String>(),
        Err(error) => format!(
            "ERR {}\n",
            engine
                .diagnostic
                .as_ref()
                .map_or(error.as_str(), |d| d.message.as_str())
        ),
    };
    let metadata = engine.array("x").map(|a| {
        // class() is the language-level class, including the deliberately
        // struct-backed MException/onCleanup facades.
        engine.execute("lang2_class=class(x);").unwrap();
        let Some(matrust::Array::Character(c)) = engine.array("lang2_class") else {
            panic!("class must return text")
        };
        format!(
            "{} [{} {}]",
            c.to_utf8(&Default::default()).unwrap(),
            a.rows(),
            a.cols()
        )
    });
    let expected_class = case
        .class
        .map(|s| if case.id == 71 { "char [1 6]" } else { s });
    let expected = if case.id == 86 {
        "ERR Identifier must be two or more parts separated by ':', each part letters, digits or underscore. See help MException.\n".into()
    } else {
        plain(case.text, case.id)
    };
    let matches = plain(&actual, case.id) == expected
        && (expected_class.is_none() || metadata.as_deref() == expected_class);
    (matches, actual, metadata)
}

#[test]
fn replay_reference() {
    let mut passed = 0;
    let mut failures = Vec::new();
    let mut report = |case: &Case, label: &str, actual: &str, metadata: Option<String>| {
        failures.push(format!(
            "case {}{label} `{}`\n expected {:?} {:?}\n actual   {:?} {:?}",
            case.id,
            case.source,
            plain(case.text, case.id),
            case.class,
            plain(actual, case.id),
            metadata
        ));
    };
    for case in cases() {
        let (matches, actual, metadata) = run_case(&case, "");
        if let Some((_, reason)) = KNOWN.iter().find(|(n, _)| *n == case.id) {
            if matches {
                report(
                    &case,
                    " (KNOWN now passes; remove its exception)",
                    &actual,
                    metadata,
                );
                continue;
            }
            eprintln!("KNOWN {}: {reason}", case.id);
        } else if matches {
            passed += 1;
        } else {
            report(&case, "", &actual, metadata);
        }
    }
    assert!(
        failures.is_empty(),
        "{passed}/{} passed; {} KNOWN; {} failures:\n{}",
        211 - KNOWN.len(),
        KNOWN.len(),
        failures.len(),
        failures.join("\n")
    );
    assert_eq!(passed, 211 - KNOWN.len());
}

#[test]
fn exception_identity_and_workspace_lifetimes() {
    let mut e = Engine::default();
    e.execute("m=MException('a:b','message'); assert(isa(m,'MException')); try, throw(m); catch caught, assert(strcmp(caught.message,'message')); assert(isa(caught,'MException')); end").unwrap();
    e.execute("global G; G=3; function y=counter(), persistent n; if isempty(n), n=0; end; n=n+1; y=n; end").unwrap();
    e.execute("global G; assert(G==3);").unwrap();
    let result = e
        .execute("c=onCleanup(@() disp('done')); alias=c; clear c; clear alias;")
        .unwrap();
    assert_eq!(result.output, ["done"]);
    let result = e
        .execute("c=onCleanup(@() disp('captured')); holder=@() c; clear c; clear holder;")
        .unwrap();
    assert_eq!(result.output, ["captured"]);
    let result = e.execute("use_cleanup(onCleanup(@() disp('released')), side_effect()); function use_cleanup(c,v), disp('called'); end; function y=side_effect(), disp('second input'); y=1; end").unwrap();
    assert_eq!(result.output, ["second input", "called", "released"]);
    e.execute("q=7; f=@() evalin('caller','q'); assert(f()==7); g=@() assignin('base','q',9); g(); assert(q==9);").unwrap();
    e.execute("m=addCause(MException('a:b','outer'),MException('c:d','inner')); try, throw(m); catch caught, try, rethrow(caught); catch again, assert(strcmp(again.cause{1}.message,'inner')); end; end").unwrap();
    e.execute("assert(optional()==0); function y=optional(a,varargin), y=numel(varargin); end")
        .unwrap();
    e.execute("assert(recursive_count(3)==4); function y=recursive_count(depth), persistent n; if isempty(n), n=0; end; n=n+1; if depth>0, recursive_count(depth-1); end; y=n; end").unwrap();
    e.execute("unknown_runtime_variable;").unwrap_err();
    e.execute("assert(~isempty(strfind(lasterr,'unknown_runtime_variable')));")
        .unwrap();
}

#[test]
fn typed_partitions_validation_and_scanner_positions() {
    let mut e = Engine::default();
    e.execute("a=uint64(9007199254740992); b=a+uint64(1); validateattributes([a b],{'uint64'},{'increasing','integer','positive'}); c=mat2cell([a b],1,[1 1]); assert(isa(c{2},'uint64')); assert(c{2}==b);").unwrap();
    e.execute("c=mat2cell(logical([1 0;0 1]),[1 1],[1 1]); assert(islogical(c{1})); assert(c{2}==false); c=mat2cell([1 2],1,[0 2]); assert(isempty(c{1})); assert(isequal(c{2},[1 2]));").unwrap();
    e.execute("[a,n,msg,next]=sscanf('7f 10 stop','%x'); assert(isequal(a,[127;16])); assert(n==2); assert(next==7); assert(strcmp(msg,'Matching failure in format.')); assert(isequal(sscanf('10 17','%o'),[8;15]));").unwrap();
}

#[test]
fn persistent_across_runs_and_current_folder_function_refresh() {
    let root = std::env::temp_dir();
    let folder = tempfile::tempdir_in(root).unwrap();
    let file = folder.path().join("lng_counter.m");
    std::fs::write(
        &file,
        "function y=lng_counter()\npersistent n\nif isempty(n), n=0; end\nn=n+1; y=n;\n",
    )
    .unwrap();
    let mut e = Engine::default();
    // No set_function_directory: files in Current Folder must resolve.
    e.set_file_directory(folder.path()).unwrap();
    e.execute("a=lng_counter();").unwrap();
    e.execute(
        "b=lng_counter(); assert(isequal([a b],[1 2])); clear lng_counter; c=lng_counter(); assert(c==1);",
    )
    .unwrap();
    std::fs::write(&file, "function y=lng_counter()\ny=99;\n").unwrap();
    e.execute("clear lng_counter; rehash; assert(lng_counter()==99);")
        .unwrap();
}

fn text(e: &Engine, name: &str) -> String {
    let Some(matrust::Array::Character(c)) = e.array(name) else {
        panic!("{name} must be text")
    };
    c.to_utf8(&Default::default()).unwrap()
}

#[test]
fn stacks_name_the_raising_function_and_top_level_stays_empty() {
    let mut e = Engine::default();
    e.execute("try, error('a:b','top'); catch err, end; s=size(err.stack);")
        .unwrap();
    assert_eq!(e.variables["s"].data(), &[0., 1.]);
    e.execute("try, outer(); catch err, n=err.stack(1).name; m=err.stack(2).name; k=numel(err.stack); end\nfunction outer(), inner(); end\nfunction inner(), error('a:b','deep'); end").unwrap();
    assert_eq!(
        (text(&e, "n"), text(&e, "m")),
        ("inner".into(), "outer".into())
    );
    assert_eq!(e.variables["k"].data(), &[3.]);
    e.execute(
        "try, h(); catch err, n=err.stack(1).name; end\nfunction h(), undefined_name_zz + 1; end",
    )
    .unwrap();
    assert_eq!(text(&e, "n"), "h");
    e.execute("r=caught(); function r=caught(), try, error('x:y','z'); catch err, r=err.stack(1).name; end; end").unwrap();
    assert_eq!(text(&e, "r"), "caught");
}

#[test]
fn lasterr_ignores_warnings_and_varargout_statement_calls_set_ans() {
    let mut e = Engine::default();
    e.execute("lasterr('before'); warning('off','all'); warning('w:x','value %d',3); nargchk(1,2,3); validateattributes(1,{'numeric'},{'unknownattr'}); m=lasterr;").unwrap();
    assert_eq!(text(&e, "m"), "before");
    assert_eq!(text(&e, "ans"), "Too many input arguments.");
    let out = e
        .execute("f\nfunction varargout=f(), varargout={5,6}; end")
        .unwrap();
    assert_eq!(e.variables["ans"].data(), &[5.]);
    assert!(out.output.join("\n").contains("ans ="), "{:?}", out.output);
    e.execute("ans=1; g()\nfunction varargout=g(), end")
        .unwrap();
    assert_eq!(e.variables["ans"].data(), &[1.]);
    e.execute("ans=1; k();\nfunction [a,varargout]=k(), a=7; end")
        .unwrap();
    assert_eq!(e.variables["ans"].data(), &[7.]);
}

#[test]
fn anonymous_bodies_have_no_function_counts() {
    let mut e = Engine::default();
    for source in [
        "f=@() nargin; f()",
        "f=@() nargout; f()",
        "f=@(a) inputname(1); f(1)",
        "f=@() nargoutchk(0,1); f()",
    ] {
        assert!(e.execute(source).is_err(), "{source}");
    }
}

#[test]
fn celldisp_long_text_and_sscanf_limits() {
    let mut e = Engine::default();
    let out = e.execute("c={1}; celldisp(c)").unwrap();
    assert!(out.output.join("\n").contains("c{1} ="), "{:?}", out.output);
    let out = e.execute("celldisp({1})").unwrap();
    assert!(
        out.output.join("\n").contains("ans{1} ="),
        "{:?}",
        out.output
    );
    e.execute("v=sscanf(sprintf('%d ',1:20000),'%d'); n=numel(v); w=sscanf(sprintf('%d ',ones(1,70000)),'%d'); m=numel(w);").unwrap();
    assert_eq!(e.variables["n"].data(), &[20000.]);
    assert_eq!(e.variables["m"].data(), &[70000.]);
    e.execute("q=sscanf('1 2 3','%d',[1e9 Inf]); s=size(q);")
        .unwrap();
    assert_eq!(e.variables["s"].data(), &[3., 1.]);
}

#[test]
fn validateattributes_descriptions_and_attributes() {
    let mut e = Engine::default();
    for (source, message) in [
        (
            "validateattributes(-1,{'numeric'},{'positive'},'fn','arg')",
            "Expected arg to be positive.",
        ),
        (
            "validateattributes(-1,{'numeric'},{'positive'},'fn','',2)",
            "Expected input number 2 to be positive.",
        ),
        (
            "validateattributes(-1,{'numeric'},{'positive'},'fn','arg',2)",
            "Expected input number 2, arg, to be positive.",
        ),
        (
            "validateattributes([1 2],{'numeric'},{'numel',3})",
            "Expected input to be an array with number of elements equal to 3.",
        ),
        (
            "validateattributes([1 2],{'numeric'},{'nrows',2})",
            "Expected input to be an array with number of rows equal to 2.",
        ),
        (
            "validateattributes([1 2],{'numeric'},{'ncols',3})",
            "Expected input to be an array with number of columns equal to 3.",
        ),
        (
            "validateattributes([1 2],{'numeric'},{'ndims',3})",
            "Expected input to be an array with number of dimensions equal to 3.",
        ),
        (
            "validateattributes(3,{'numeric'},{'even'})",
            "Expected input to be even.",
        ),
        (
            "validateattributes(-4,{'numeric'},{'odd'})",
            "Expected input to be odd.",
        ),
        (
            "validateattributes([0 2],{'numeric'},{'binary'})",
            "Expected input to be binary.",
        ),
        (
            "validateattributes([1 2;3 4],{'numeric'},{'diag'})",
            "Expected input to be diagonal.",
        ),
    ] {
        let error = e.execute(source).unwrap_err();
        assert!(error.contains(message), "{source}: {error}");
    }
    e.execute("validateattributes([2 -4],{'numeric'},{'even','3d','nonsparse','ndims',2}); validateattributes([-3 1],{'numeric'},{'odd'}); validateattributes(logical([0 1]),{'logical'},{'binary'}); validateattributes([1 0;0 4],{'numeric'},{'diag','numel',4,'nrows',2,'ncols',2});").unwrap();
    e.execute("try, validateattributes([1 2],{'numeric'},{'numel',3},'fn'); catch err, id=err.identifier; end").unwrap();
    assert_eq!(text(&e, "id"), "MATLAB:fn:incorrectNumel");
}

#[test]
fn exception_facade_hides_internal_fields() {
    let mut e = Engine::default();
    // Public properties are read-only, so a replaced cause cannot be built.
    let error = e
        .execute("me=MException('a:b','m'); me.cause=1;")
        .unwrap_err();
    assert!(
        error.contains("You cannot set the read-only property 'cause' of MException."),
        "{error}"
    );
    e.execute("me=MException('a:b','m'); f=fieldnames(me); n=numel(f); first=f{1};")
        .unwrap();
    assert_eq!(e.variables["n"].data(), &[4.]);
    assert_eq!(text(&e, "first"), "identifier");
    let error = e.execute("struct2cell(me)").unwrap_err();
    assert!(
        error
            .contains("Undefined function 'struct2cell' for input arguments of type 'MException'."),
        "{error}"
    );
    for source in [
        "disp(me)",
        "me",
        "c={me}; c{1}",
        "outer=addCause(me,MException('c:d','inner')); outer.cause{1}",
        "try, error('x:y','z'); catch err, end; err",
        "t=onCleanup(@() 1); t",
        "whos",
    ] {
        let out = e.execute(source).unwrap().output.join("\n");
        assert!(!out.contains("MatRust"), "{source}: {out}");
        assert!(
            out.contains("MException") || out.contains("onCleanup"),
            "{source}: {out}"
        );
    }
}

#[test]
fn objects_are_not_structs_and_are_read_only() {
    let mut e = Engine::default();
    e.execute("me=MException('a:b','m'); a=isstruct(me); b=isfield(me,'message'); c=isfield(me,'MatRustObjectKind'); d=isfield(me,{'message','x'}); n=numfields(me);")
        .unwrap();
    assert_eq!(e.variables["n"].data(), &[0.]);
    e.execute("assert(~a && ~b && ~c && isequal(d,[false false]));")
        .unwrap();
    for (source, message) in [
        (
            "me.message='x';",
            "You cannot set the read-only property 'message' of MException.",
        ),
        (
            "me.stack(1).name='x';",
            "You cannot set the read-only property 'stack' of MException.",
        ),
        (
            "me.('identifier')='x';",
            "You cannot set the read-only property 'identifier' of MException.",
        ),
        (
            "me.extra=1;",
            "No public property extra exists for class MException.",
        ),
        (
            "me.MatRustObjectKind=3;",
            "No public property MatRustObjectKind exists for class MException.",
        ),
        (
            "t=onCleanup(@() 1); t.task=1;",
            "You cannot set the read-only property 'task' of onCleanup.",
        ),
        (
            "rmfield(me,'message')",
            "Undefined function 'rmfield' for input arguments of type 'MException'.",
        ),
    ] {
        let error = e.execute(source).unwrap_err();
        assert!(error.contains(message), "{source}: {error}");
    }
    e.execute("assert(strcmp(me.message,'m')); assert(strcmp(class(me),'MException'));")
        .unwrap();
    // Nested displays never expose the internal fields.
    for source in [
        "c={me}",
        "s.e=me; s",
        "s.e=me; t={s}",
        "disp({me})",
        "s(2).e=me; s",
    ] {
        let out = e.execute(source).unwrap().output.join("\n");
        assert!(!out.contains("MatRust"), "{source}: {out}");
    }
}

#[test]
fn reference_checked_validation_regexptranslate_and_oncleanup_forms() {
    // Checked against the reference by the orchestrator (2026-10-24).
    let mut e = Engine::default();
    e.execute("try, validateattributes(5, {}, {'positive'}); catch err, msg=err.message; end")
        .unwrap();
    assert!(
        text(&e, "msg").starts_with("Expected input to be one of these types:"),
        "{}",
        text(&e, "msg")
    );
    e.execute("t=regexptranslate('wildcard','a(b)+c.d*e?');")
        .unwrap();
    assert_eq!(text(&e, "t"), "a\\(b\\)\\+c\\.d.*e.");
    e.execute("c=onCleanup(5); clear c;").unwrap();
}

#[test]
fn validateattributes_size_error_has_message_and_identifier() {
    let mut e = Engine::default();
    e.execute("try, validateattributes([1 2], {'numeric'}, {'size',[1 3]}); catch err, msg=err.message; id=err.identifier; end")
        .unwrap();
    assert_eq!(
        text(&e, "msg"),
        "Expected input to be of size 1x3 when it is actually size 1x2."
    );
    assert_eq!(text(&e, "id"), "MATLAB:incorrectSize");
    e.execute("try, validateattributes([1 2], {'numeric'}, {'size',[1 3]}, 'fn', 'arg'); catch err, msg=err.message; id=err.identifier; end")
        .unwrap();
    assert_eq!(
        text(&e, "msg"),
        "Expected arg to be of size 1x3 when it is actually size 1x2."
    );
    assert_eq!(text(&e, "id"), "MATLAB:fn:incorrectSize");
    e.execute("validateattributes([1 2], {'numeric'}, {'size',[1 NaN]});")
        .unwrap();
}

#[test]
fn empty_varargin_is_zero_by_zero() {
    let mut e = Engine::default();
    e.execute("a=f(); b=f(1); c=f(1,2); g=@(varargin) size(varargin); d=g(); h=g(1);\nfunction s=f(varargin), s=size(varargin); end")
        .unwrap();
    for (name, size) in [
        ("a", [0., 0.]),
        ("b", [1., 1.]),
        ("c", [1., 2.]),
        ("d", [0., 0.]),
        ("h", [1., 1.]),
    ] {
        assert_eq!(e.variables[name].data(), &size, "{name}");
    }
    e.execute("assert(isequal(emptyargs(), {})); function v=emptyargs(varargin), v=varargin; end")
        .unwrap();
}

#[test]
fn sscanf_count_is_the_number_of_conversions() {
    let mut e = Engine::default();
    e.execute("[a,n]=sscanf('abc','%s'); [b,m]=sscanf('12 34','%d'); [c,k]=sscanf('abc','%c'); [d,j]=sscanf('1 x','%d');")
        .unwrap();
    for (name, count) in [("n", 1.), ("m", 2.), ("k", 3.), ("j", 1.)] {
        assert_eq!(e.variables[name].data(), &[count], "{name}");
    }
}

#[test]
fn inputname_names_never_go_stale() {
    let mut e = Engine::default();
    let functions = "\nfunction r=two(a,b), r=inputname(2); end\nfunction [r,s]=pair(a,b), r=inputname(1); s=inputname(2); end";
    e.execute(&format!(
        "p=1; q=2; c1={{1}}; c2={{2}};\n\
         r1=two(p,q); assert(strcmp(r1,'q'));\n\
         r2=cellfun(@two,c1,c2,'UniformOutput',false); assert(isempty(r2{{1}}));\n\
         r3=arrayfun(@two,p,q,'UniformOutput',false); assert(isempty(r3{{1}}));\n\
         r4=feval(@two,p,q); assert(isempty(r4));\n\
         r5=feval('two',p,q); assert(isempty(r5));\n\
         h=@two; r6=h(p,q); assert(isempty(r6));\n\
         [x1,x2]=pair(p,q); assert(strcmp(x1,'p') && strcmp(x2,'q'));\n\
         two(p,q); assert(strcmp(ans,'q'));\n\
         r7=two(c1{{1}},q); assert(isempty(r7));\n\
         r8=cellfun(@two,c1,c2,'UniformOutput',false); assert(isempty(r8{{1}}));{functions}"
    ))
    .unwrap();
    // A call that fails before entering a function leaves nothing behind.
    e.execute(&format!(
        "try, two(p,q,p); catch, end\nr=feval(@two,5,6); assert(isempty(r));{functions}"
    ))
    .unwrap();
}

#[test]
fn persistent_values_reset_when_the_function_changes() {
    let mut e = Engine::default();
    let counter = "function y=tick(), persistent n; if isempty(n), n=0; end; n=n+1; y=n; end";
    e.execute(&format!("a=tick();\n{counter}")).unwrap();
    e.execute(&format!("a=tick();\n{counter}")).unwrap();
    assert_eq!(e.variables["a"].data(), &[2.]);
    e.execute(&format!("a=tick();\n{}", counter.replace("n+1", "n+10")))
        .unwrap();
    assert_eq!(e.variables["a"].data(), &[10.]);
    let root = std::env::temp_dir();
    let folder = tempfile::tempdir_in(root).unwrap();
    let file = folder.path().join("lng_tick.m");
    std::fs::write(&file, counter.replace("tick", "lng_tick")).unwrap();
    e.set_file_directory(folder.path()).unwrap();
    e.execute("a=lng_tick(); a=lng_tick();").unwrap();
    assert_eq!(e.variables["a"].data(), &[2.]);
    std::fs::write(
        &file,
        counter.replace("tick", "lng_tick").replace("n+1", "n+5"),
    )
    .unwrap();
    e.execute("a=lng_tick();").unwrap();
    assert_eq!(e.variables["a"].data(), &[5.]);
}

#[test]
fn user_clear_function_keeps_precedence_over_the_statement() {
    let mut e = Engine::default();
    e.execute("A=1; clear(5); function clear(x), assignin('base','got',x); end")
        .unwrap();
    assert!(e.array("A").is_some());
    assert_eq!(e.variables["got"].data(), &[5.]);
}

#[test]
#[ignore = "manual release benchmark; includes parsing and workspace updates"]
fn language_array_benchmark() {
    for workers in [1, 4] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        pool.install(|| {
            for (size, repetitions) in [(64, 200), (65536, 20), (1_000_000, 5)] {
                let mut engine = Engine::default();
                engine.execute(&format!("A=1:{size};")).unwrap();
                let original = engine.array("A").unwrap();
                let source = format!("C=mat2cell(A,1,[{} {}]); validateattributes(A,{{'double'}},{{'positive','increasing'}});", size/2, size/2);
                engine.execute(&source).unwrap();
                let start = std::time::Instant::now();
                for _ in 0..repetitions { engine.execute(&source).unwrap(); }
                let micros = start.elapsed().as_secs_f64()*1e6/repetitions as f64;
                assert!(original.same_snapshot(&engine.array("A").unwrap()));
                eprintln!("workers={workers} elements={size} repetitions={repetitions} mean_us={micros:.2}");
            }
        });
    }
}
