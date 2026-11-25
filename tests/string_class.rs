//! Expectations follow the task's string semantics; no MATLAB-derived fixtures.
use matrust::{Array, Engine, StringArray};
use std::sync::{Arc, atomic::Ordering};

#[test]
fn merged_courses_dispatch_keeps_string_array_guard() {
    for source in [
        "[p,f]=periodogram([\"1\" \"2\"]);",
        "periodogram([\"1\" \"2\"]);",
        "[K,S,P]=lqr(-1,[\"1\" \"2\"],1,eye(2));",
        "[p,loc]=findpeaks([\"1\" \"2\"]);",
        "h=@ctrb; x=h([\"1\" \"2\"],1);",
    ] {
        let error = Engine::default().execute(source).unwrap_err();
        assert!(
            error.contains("does not support string arrays yet"),
            "{source}: {error}"
        );
    }
}

fn run(code: &str) -> Engine {
    let mut e = Engine::default();
    e.execute(code)
        .unwrap_or_else(|err| panic!("{code}: {err}"));
    e
}
fn strings(e: &Engine, name: &str, rows: usize, cols: usize, expected: &[Option<&str>]) {
    let Array::Str(s) = e.array(name).unwrap() else {
        panic!("{name}: expected string")
    };
    assert_eq!((s.rows(), s.cols()), (rows, cols), "{name}");
    assert_eq!(
        s.data().iter().map(|s| s.as_deref()).collect::<Vec<_>>(),
        expected,
        "{name}"
    );
}
fn text(e: &Engine, name: &str, expected: &str) {
    strings(e, name, 1, 1, &[Some(expected)]);
}
fn numeric(e: &Engine, name: &str, expected: &[f64]) {
    let a = e.array(name).unwrap();
    let m = a.to_numeric(&e.cancellation()).unwrap();
    assert_eq!(m.len(), expected.len(), "{name}");
    for (a, b) in m.data().iter().zip(expected) {
        assert!(a == b || a.is_nan() && b.is_nan(), "{name}: {a} != {b}");
    }
}
fn chars(e: &Engine, name: &str, expected: &str) {
    let Array::Character(c) = e.array(name).unwrap() else {
        panic!("{name}: expected char")
    };
    assert_eq!(
        c.units(),
        expected.encode_utf16().collect::<Vec<_>>(),
        "{name}"
    );
}
fn cellstr(e: &Engine, name: &str, shape: (usize, usize), expected: &[&str]) {
    let Array::Cell(c) = e.array(name).unwrap() else {
        panic!("{name}: expected cellstr")
    };
    assert_eq!((c.rows(), c.cols()), shape, "{name}");
    assert_eq!(c.len(), expected.len());
    for (i, expected) in expected.iter().enumerate() {
        let Array::Character(s) = c.get_linear(i).unwrap().as_ref() else {
            panic!("{name}: expected char slot")
        };
        assert_eq!(
            s.units(),
            expected.encode_utf16().collect::<Vec<_>>(),
            "{name}({i})"
        );
    }
}

#[test]
fn round5_01_recursive_equality_compares_strings_and_missing() {
    for (equal, missing) in [("isequal", 0.), ("isequaln", 1.)] {
        let e = run(&format!(
            r#"c1={{"a",1}}; c2={{"a",1}}; s1.a="x"; s2.a="x";
            same=[{equal}(c1,c2) {equal}(s1,s2) {equal}({{"a"}},{{"a"}},{{"a"}})];
            nested1.a={{{{["a" "b";"c" "d"]}}}};
            nested2.a={{{{["a" "b";"c" "d"]}}}};
            nested={equal}(nested1,nested2);
            missing1.a={{{{["a" missing]}}}}; missing2.a={{{{["a" missing]}}}};
            absent=[{equal}({{missing}},{{missing}}) {equal}(missing1,missing2) {equal}(missing1,missing1)];
            mismatch1.a="a"; mismatch2.a='a'; mismatch3.a="b";
            different=[{equal}({{"a"}},{{'a'}}) {equal}({{"a"}},{{"b"}})
                {equal}(mismatch1,mismatch2) {equal}(mismatch1,mismatch3)
                {equal}({{["a" "b"]}},{{["a";"b"]}}) {equal}({{missing}},{{""}})
                {equal}({{"a"}},{{"a"}},{{"b"}}) {equal}({{strings(0,2)}},{{strings(0,3)}})];
            empty={equal}({{strings(0,2)}},{{strings(0,2)}});"#
        ));
        numeric(&e, "same", &[1., 1., 1.]);
        numeric(&e, "nested", &[1.]);
        numeric(&e, "absent", &[missing; 3]);
        numeric(&e, "different", &[0.; 8]);
        numeric(&e, "empty", &[1.]);
    }
}

#[test]
fn round5_02_concatenation_preserves_shaped_empty_operands() {
    let e = run(
        r#"h=[strings(0,2) strings(0,3)]; v=[strings(2,0);strings(3,0)];
        mixedh=[strings(0,2) zeros(0,3)]; mixedv=[zeros(2,0);strings(3,0)];
        horizontal=["a" strings(1,0)]; vertical=[strings(0,1);"a"];
        numeric_shapes=[size([zeros(0,2) zeros(0,3)]) size([zeros(2,0);zeros(3,0)])];"#,
    );
    strings(&e, "h", 0, 5, &[]);
    strings(&e, "v", 5, 0, &[]);
    strings(&e, "mixedh", 0, 5, &[]);
    strings(&e, "mixedv", 5, 0, &[]);
    text(&e, "horizontal", "a");
    text(&e, "vertical", "a");
    numeric(&e, "numeric_shapes", &[0., 5., 5., 0.]);
    for (string_code, numeric_code, dimension) in [
        (r#"["a" strings(0,2)]"#, "[1 zeros(0,2)]", "row"),
        (r#"["a" zeros(0,2)]"#, "[1 zeros(0,2)]", "row"),
        (r#"[strings(2,0);"a"]"#, "[zeros(2,0);1]", "column"),
        (r#"[zeros(2,0);"a"]"#, "[zeros(2,0);1]", "column"),
        (
            "[strings(0,2);strings(0,3)]",
            "[zeros(0,2);zeros(0,3)]",
            "column",
        ),
        (
            "[strings(2,0) strings(3,0)]",
            "[zeros(2,0) zeros(3,0)]",
            "row",
        ),
    ] {
        let mut e = Engine::default();
        let error = e.execute(string_code).unwrap_err();
        assert!(
            error.contains(&format!("{dimension} dimensions disagree")),
            "{string_code}: {error}"
        );
        assert!(e.execute(numeric_code).is_err(), "{numeric_code}");
    }
}

#[test]
fn round4_01_checkpoint_skips_strings_and_nested_cells_and_restores_numeric() {
    use matrust::{
        document::Document,
        session::{Session, SessionStore, workspace},
        simulation::Model,
    };
    let e = run(
        "keep=42; name=\"x\"; many=[\"a\" \"b\"]; nested={{name}}; numeric_cell={keep}; st.f=1;",
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    let state = Session {
        layout_presets: Default::default(),
        export: Default::default(),
        workspace: Some(e.workspace()),
        workspace_manifest: None,
        preview: None,
        eps_renderer: None,
        figure_manifest: None,
        figures: None,
        layout: Default::default(),
        version: 1,
        documents: vec![Document::new()],
        active: 0,
        folder: std::env::current_dir().unwrap(),
        recent: vec![],
        history: vec![],
        model: Model::default(),
        model_path: None,
        model_baseline: None,
    };
    SessionStore::new(&path).save(state).unwrap();
    let manifest = Session::load(&path).unwrap().workspace_manifest.unwrap();
    assert_eq!(manifest.skipped, ["many", "name", "nested", "st"]);
    assert!(manifest.skipped_note().unwrap().contains("strings"));
    let restored = workspace::restore(&path, &manifest, &e.cancellation()).unwrap();
    assert_eq!(restored.len(), 2);
    assert_eq!(
        restored["keep"]
            .to_numeric(&e.cancellation())
            .unwrap()
            .as_scalar()
            .unwrap(),
        42.
    );
    let Array::Cell(c) = &restored["numeric_cell"] else {
        panic!("expected cell")
    };
    assert_eq!(
        c.get_linear(0)
            .unwrap()
            .to_numeric(&e.cancellation())
            .unwrap()
            .as_scalar()
            .unwrap(),
        42.
    );
}

#[test]
fn round4_02_string_formats_accept_all_integer_classes_and_single() {
    for class in [
        "int8", "uint8", "int16", "uint16", "int32", "uint32", "int64", "uint64", "single",
    ] {
        let e = run(&format!(
            "x={class}([5 7;6 8]); a=sprintf(\"%d,\",x); b=sprintf('%d,',x); c=compose(\"%d\",x); d=compose('%d',x); scalar=compose(\"%d\",{class}(5));"
        ));
        text(&e, "a", "5,6,7,8,");
        chars(&e, "b", "5,6,7,8,");
        strings(&e, "c", 2, 2, &[Some("5"), Some("6"), Some("7"), Some("8")]);
        cellstr(&e, "d", (2, 2), &["5", "6", "7", "8"]);
        text(&e, "scalar", "5");
    }
    let e = run("a=sprintf(\"%.2f\",single(1.25)); b=compose(\"%.2f\",single([1.25 2.5]));");
    text(&e, "a", "1.25");
    strings(&e, "b", 1, 2, &[Some("1.25"), Some("2.50")]);
}

#[test]
fn round4_03_pad_counts_utf16_units() {
    let e = run(
        "a=pad(\"😀\",4); b=pad(\"😀\",4,'left'); c=pad(\"😀\",4,'both'); d=pad(\"😀\",1); n=strlength([a b c d]); p=pad([\"😀\" \"a😀\"],4,'-');",
    );
    text(&e, "a", "😀  ");
    text(&e, "b", "  😀");
    text(&e, "c", " 😀 ");
    text(&e, "d", "😀");
    numeric(&e, "n", &[4., 4., 4., 2.]);
    strings(&e, "p", 1, 2, &[Some("😀--"), Some("a😀-")]);
}

#[test]
fn review_01_missing_is_builtin_and_nosuchfn_sentinel_remains_an_error_sentinel() {
    let mut e = run("m=missing; q=ismissing(m);");
    numeric(&e, "q", &[1.]);
    for code in [
        "y=nosuchfn_sentinel;",
        "nosuchfn_sentinel;",
        "bad=nosuchfn_sentinel(1);",
    ] {
        let err = e.execute(code).unwrap_err();
        assert!(err.contains("nosuchfn_sentinel"), "{err}");
    }
}

#[test]
fn review_02_text_functions_preserve_char_cellstr_and_string_types() {
    for (expression, expected) in [
        ("strip(%s)", "ab"),
        ("erase(%s,'a')", " b "),
        ("replace(%s,'a','z')", " zb "),
        ("extractBefore(%s,3)", " a"),
        ("extractAfter(%s,2)", "b "),
        ("extractBetween(%s,2,3)", "ab"),
        ("insertBefore(%s,2,'!')", " !ab "),
        ("insertAfter(%s,2,'!')", " a!b "),
        ("pad(%s,5)", " ab  "),
        ("reverse(%s)", " ba "),
        ("upper(%s)", " AB "),
        ("lower(%s)", " ab "),
    ] {
        let e = run(&format!(
            "a={}; b={}; c={};",
            expression.replace("%s", "' ab '"),
            expression.replace("%s", "{' ab '}"),
            expression.replace("%s", "\" ab \"")
        ));
        chars(&e, "a", expected);
        cellstr(&e, "b", (1, 1), &[expected]);
        text(&e, "c", expected);
    }
    let e =
        run("a=split('a b'); b=compose('x=%d',[1 2]); c=join('abc','-'); d=join({'a','b'},'-');");
    cellstr(&e, "a", (2, 1), &["a", "b"]);
    cellstr(&e, "b", (1, 2), &["x=1", "x=2"]);
    chars(&e, "c", "abc");
    cellstr(&e, "d", (1, 1), &["a-b"]);
}

#[test]
fn review_03_ismissing_distinguishes_numeric_cellstr_and_string_missing() {
    let mut e = run(
        r#"a=ismissing([1 NaN 3]); b=ismissing({'a',''}); c=ismissing(["" missing]); d=ismissing(single([1 NaN])); f=ismissing(int32([1 2]));"#,
    );
    numeric(&e, "a", &[0., 1., 0.]);
    numeric(&e, "b", &[0., 1.]);
    numeric(&e, "c", &[0., 1.]);
    numeric(&e, "d", &[0., 1.]);
    numeric(&e, "f", &[0., 0.]);
    e.execute("g=ismissing(reshape([1 NaN 3 NaN],1,2,2));")
        .unwrap();
    assert_eq!(e.array("g").unwrap().dims().as_ref(), &[1, 2, 2]);
    numeric(&e, "g", &[0., 1., 0., 1.]);
    for code in [
        "ismissing('abc')",
        "ismissing(true)",
        "ismissing({1,''})",
        "ismissing(struct())",
    ] {
        assert!(e.execute(code).unwrap_err().contains("ismissing"), "{code}");
    }
}

#[test]
fn review_04_disp_and_display_keep_array_quotes_without_a_header() {
    for function in ["disp", "display"] {
        let mut e = Engine::default();
        assert!(
            e.execute(&format!(r#"{function}("");"#))
                .unwrap()
                .output
                .is_empty()
        );
        assert_eq!(
            e.execute(&format!(r#"{function}("abc");"#)).unwrap().output,
            ["abc"]
        );
        assert_eq!(
            e.execute(&format!(r#"{function}(["a" "b"]);"#))
                .unwrap()
                .output,
            ["    \"a\"    \"b\""]
        );
        assert_eq!(
            e.execute(&format!(r#"{function}(["a";missing]);"#))
                .unwrap()
                .output,
            ["    \"a\"\n    <missing>"]
        );
    }
}

#[test]
fn review_05_patterns_are_lists_applied_to_every_subject() {
    let e = run(
        r#"a=contains("abc",["x" "b"]); b=startsWith(["abc";"xbc"],["x" "a"]); c=endsWith(["ab" "cd" "ef"],["b";"f"]); d=replace("abc",["a" "b"],["x" "y"]); f=strrep(["abc";"bca"],["a" "b"],["x" "y"]); g=regexprep("abc",["a" "b"],["x" "y"]); h=erase("abc",["a" "b"]); j=replace("ab",["a" "b"],["b" "x"]); k=contains("ABC",[missing "b"],'IgnoreCase',true); l=erase('abc',{'a','b'});"#,
    );
    numeric(&e, "a", &[1.]);
    numeric(&e, "b", &[1., 1.]);
    numeric(&e, "c", &[1., 0., 1.]);
    assert_eq!(e.array("b").unwrap().dims().as_ref(), &[2, 1]);
    assert_eq!(e.array("c").unwrap().dims().as_ref(), &[1, 3]);
    text(&e, "d", "xyc");
    strings(&e, "f", 2, 1, &[Some("xyc"), Some("ycx")]);
    text(&e, "g", "xyc");
    text(&e, "h", "c");
    text(&e, "j", "xx");
    numeric(&e, "k", &[1.]);
    chars(&e, "l", "c");
}

#[test]
fn review_06_empty_char_shapes_convert_to_one_empty_string() {
    let mut e = run(r#"x=''; a="Name: "+x; b=["a" x]; c=string(x);"#);
    text(&e, "a", "Name: ");
    strings(&e, "b", 1, 2, &[Some("a"), Some("")]);
    text(&e, "c", "");
    for code in ["empty=char(zeros(0,3));", "empty=char(zeros(3,0));"] {
        e.execute(code).unwrap();
        e.execute("converted=string(empty);").unwrap();
        text(&e, "converted", "");
    }
}

#[test]
fn review_07_switch_matches_char_and_scalar_string_cases() {
    for (subject, case) in [
        ("\"b\"", "'b'"),
        ("'b'", "\"b\""),
        ("\"b\"", "\"b\""),
        ("'b'", "{\"b\",\"c\"}"),
        ("\"b\"", "{'b','c'}"),
    ] {
        let e = run(&format!(
            "x={subject}; result=0; switch x; case {case}; result=7; otherwise; result=9; end"
        ));
        numeric(&e, "result", &[7.]);
    }
    assert!(
        Engine::default()
            .execute(r#"switch ["a" "b"]; case "b"; end"#)
            .unwrap_err()
            .contains("SWITCH expression must be a scalar")
    );
}

#[test]
fn review_08_scalar_string_equality_and_unsupported_array_consumers() {
    let mut e = run(
        r#"a=isequaln("a","a"); b=isequaln("a",'a'); c=isequaln(["a" missing],["a" missing]);"#,
    );
    numeric(&e, "a", &[1.]);
    numeric(&e, "b", &[0.]);
    numeric(&e, "c", &[1.]);
    for (name, code) in [
        ("full", r#"full("a")"#),
        ("shiftdim", r#"[a,b]=shiftdim("a")"#),
        ("repelem", r#"repelem("a",1)"#),
        ("mat2cell", r#"mat2cell("a",1,1)"#),
        (
            "validateattributes",
            r#"validateattributes("a",{'string'},{})"#,
        ),
        ("ismembertol", r#"ismembertol(1,"a")"#),
        ("permute", r#"permute("a",[1 2])"#),
        ("onCleanup", r#"onCleanup("a")"#),
        ("substruct", r#"substruct('.',"a")"#),
        ("assignin", r#"assignin('base','x',"a")"#),
    ] {
        let err = e.execute(code).unwrap_err();
        assert!(
            err.contains(&format!(
                "{name} does not support string arrays yet; convert with char(...) or cellstr(...)"
            )),
            "{code}: {err}"
        );
    }
    e.execute(r#"options=optimset('Display',"off"); display_mode=options.Display;"#)
        .unwrap();
    text(&e, "display_mode", "off");
    e.execute(r#"s=sort([2 1],"descend"); f=sprintf('%s',"ok"); plot(1:3,"LineWidth",2);"#)
        .unwrap();
    numeric(&e, "s", &[2., 1.]);
    chars(&e, "f", "ok");
    e.execute(r#"r=repmat("ab",1,2); [sorted,idx]=sort("ba"); [row,ri]=sortrows("ba"); reshaped=reshape("ab",1,2);"#).unwrap();
    chars(&e, "r", "abab");
    chars(&e, "sorted", "ab");
    numeric(&e, "idx", &[2., 1.]);
    chars(&e, "row", "ba");
    chars(&e, "reshaped", "ab");
}

#[test]
fn review_09_legacy_scalar_mat2str_and_int2str() {
    let mut e = run(r#"a=mat2str("a"); b=int2str("a");"#);
    chars(&e, "a", "'a'");
    chars(&e, "b", "97");
    assert!(
        e.execute(r#"num2str("a")"#)
            .unwrap_err()
            .contains("does not accept string inputs")
    );
}

#[test]
fn review_10_string_transposes_preserve_elements_and_missing() {
    let e = run(r#"s=["a" "b";"c" missing]; a=s'; b=s.'; c=s(:)'; scalar="abc"; d=scalar';"#);
    strings(&e, "a", 2, 2, &[Some("a"), Some("b"), Some("c"), None]);
    strings(&e, "b", 2, 2, &[Some("a"), Some("b"), Some("c"), None]);
    strings(&e, "c", 1, 4, &[Some("a"), Some("c"), Some("b"), None]);
    text(&e, "d", "abc");
}

#[test]
fn review_11_split_collapses_delimiters_and_rejects_a_third_dimension() {
    let mut e = run(
        r#"a=split("a,,b",","); b=split("a,,b",",",'CollapseDelimiters',false); c=split('a  b','CollapseDelimiters',false); d=split(["a,,b";"c,d"],",");"#,
    );
    strings(&e, "a", 2, 1, &[Some("a"), Some("b")]);
    strings(&e, "b", 3, 1, &[Some("a"), Some(""), Some("b")]);
    cellstr(&e, "c", (3, 1), &["a", "", "b"]);
    strings(&e, "d", 2, 2, &[Some("a"), Some("c"), Some("b"), Some("d")]);
    assert!(
        e.execute(r#"split(["a:b" "c:d"],':')"#)
            .unwrap_err()
            .contains("N-D string storage")
    );
}

#[test]
fn review_12_strip_and_pad_accept_the_character_shorthand() {
    let e = run(
        r#"a=strip("--a--",'-'); b=pad("a",4,'-'); c=strip('--b--','-'); d=pad({'a','bb'},4,'-');"#,
    );
    text(&e, "a", "a");
    text(&e, "b", "a---");
    chars(&e, "c", "b");
    cellstr(&e, "d", (1, 2), &["a---", "bb--"]);
}

#[test]
fn review_13_insert_text_at_every_match() {
    let e = run(
        r#"a=insertAfter("a.b.c",".","!"); b=insertBefore("a.b.c",".","!"); c=insertAfter("aaaa","aa","!");"#,
    );
    text(&e, "a", "a.!b.!c");
    text(&e, "b", "a!.b!.c");
    text(&e, "c", "aa!aa!");
}

#[test]
fn review_14_strjoin_unescapes_its_delimiter() {
    let e =
        run(r#"a=strjoin(["a" "b"],'\n'); b=strjoin(["a" "b"],"\t"); c=strjoin({'a','b'},'\n');"#);
    text(&e, "a", "a\nb");
    text(&e, "b", "a\tb");
    chars(&e, "c", "a\nb");
}

#[test]
fn review_15_text_positions_and_lengths_use_utf16_units() {
    let mut e = run(
        r#"n=strlength("😀"); a=extractBefore("😀b",3); b=extractAfter("😀b",2); c=extractBetween("x😀b",2,3); d=insertBefore("😀b",3,"!"); f=insertAfter("😀b",2,"!");"#,
    );
    numeric(&e, "n", &[2.]);
    text(&e, "a", "😀");
    text(&e, "b", "b");
    text(&e, "c", "😀");
    text(&e, "d", "😀!b");
    text(&e, "f", "😀!b");
    assert!(
        e.execute(r#"extractBefore("😀b",2)"#)
            .unwrap_err()
            .contains("surrogate pair")
    );
}

#[test]
fn review_16_single_conversion_uses_the_f32_shortest_round_trip() {
    let e = run("a=string(single([0.1 1.234567 1e-5 Inf NaN -0]));");
    strings(
        &e,
        "a",
        1,
        6,
        &[
            Some("0.1"),
            Some("1.234567"),
            Some("1e-05"),
            Some("Inf"),
            Some("NaN"),
            Some("0"),
        ],
    );
}

#[test]
fn review_17_compact_string_headers_have_no_extra_blank_line() {
    let mut e = Engine::default();
    let compact = e
        .execute(r#"format compact; s=["a" "b"]"#)
        .unwrap()
        .output
        .join("\n");
    assert!(
        compact.contains("1\u{00d7}2 string\n    \"a\""),
        "{compact:?}"
    );
    let loose = e
        .execute(r#"format loose; s=["a" "b"]"#)
        .unwrap()
        .output
        .join("\n");
    assert!(
        loose.contains("1\u{00d7}2 string\n\n    \"a\""),
        "{loose:?}"
    );
}

#[test]
fn review_18_concatenation_skips_only_zero_by_zero_string_arrays() {
    let e = run(
        r#"a=[strings(0,0) "a"]; b=["a" strings(0,0)]; c=[strings(0,0); "a"]; d=[strings(0,0) strings(0,0)];"#,
    );
    text(&e, "a", "a");
    text(&e, "b", "a");
    text(&e, "c", "a");
    strings(&e, "d", 0, 0, &[]);
}

#[test]
fn review_19_ambiguous_linear_matrix_growth_errors_atomically() {
    let mut e = run(r#"s=["a" "b";"c" "d"];"#);
    assert!(
        e.execute(r#"s(5)="x";"#)
            .unwrap_err()
            .contains("Cannot grow a matrix through one ambiguous linear subscript")
    );
    strings(&e, "s", 2, 2, &[Some("a"), Some("c"), Some("b"), Some("d")]);
}

#[test]
fn construction_queries_and_conversion() {
    let e = run(
        r#"s=["a" "b"; "c" "d"]; mixed=["a" 'bc']; a=string('abc'); c=string({'ab','c'}); n=string([3.5 1e-5 1/3 Inf NaN -0]); b=string([true false]); z=strings(0,0); blank=strings(2,3); cls=class(s); q=[isstring(s) isa(s,'string') ischar(s) iscell(s) isempty(z) isscalar(a) isrow(a) iscolumn(a) isvector(a) ismatrix(s) ndims(s) numel(s) length(s)]; sz=size(s); [r,c2,d]=size(s); l=strlength(["ab" "😀"; "" missing]); eq=[isequal("a","a") isequal("a",'a') isequal(["a" "b"],["a" "b"])]; chars=char(["a" "bbb"]); cells=cellstr(s); nums=double(["3.5" "bad" missing]); nums2=str2double(["1e-5" "-Inf"]);"#,
    );
    strings(&e, "s", 2, 2, &[Some("a"), Some("c"), Some("b"), Some("d")]);
    strings(&e, "mixed", 1, 2, &[Some("a"), Some("bc")]);
    text(&e, "a", "abc");
    strings(&e, "c", 1, 2, &[Some("ab"), Some("c")]);
    strings(
        &e,
        "n",
        1,
        6,
        &[
            Some("3.5"),
            Some("1e-05"),
            Some("0.3333333333333333"),
            Some("Inf"),
            Some("NaN"),
            Some("0"),
        ],
    );
    strings(&e, "b", 1, 2, &[Some("true"), Some("false")]);
    strings(&e, "z", 0, 0, &[]);
    strings(&e, "blank", 2, 3, &[Some(""); 6]);
    numeric(
        &e,
        "q",
        &[1., 1., 0., 0., 1., 1., 1., 1., 1., 1., 2., 4., 2.],
    );
    numeric(&e, "sz", &[2., 2.]);
    numeric(&e, "r", &[2.]);
    numeric(&e, "c2", &[2.]);
    numeric(&e, "d", &[1.]);
    numeric(&e, "l", &[2., 0., 2., f64::NAN]);
    numeric(&e, "eq", &[1., 0., 1.]);
    let Array::Character(c) = e.array("chars").unwrap() else {
        panic!()
    };
    assert_eq!((c.rows(), c.cols()), (2, 3));
    assert_eq!(c.units(), &[97, 98, 32, 98, 32, 98]);
    let Array::Cell(c) = e.array("cells").unwrap() else {
        panic!()
    };
    assert_eq!(c.len(), 4);
    assert!(matches!(
        c.get_linear(0).unwrap().as_ref(),
        Array::Character(_)
    ));
    numeric(&e, "nums", &[3.5, f64::NAN, f64::NAN]);
    numeric(&e, "nums2", &[1e-5, f64::NEG_INFINITY]);
}
#[test]
fn indexing_growth_deletion_and_aliases() {
    let mut e = run(
        r#"s=["a" "b";"c" "d"]; alias=s; x=s(3); y=s(2,end); mask=s([true false;false true]); col=s(:,2); chars=s{2}; s(2)='changed'; s(3,3)="z"; holes=ismissing(s); v=["a" "b"]; v(4)="d"; v(2)=[]; v(end+1)='e';"#,
    );
    text(&e, "x", "b");
    text(&e, "y", "d");
    strings(&e, "mask", 2, 1, &[Some("a"), Some("d")]);
    strings(&e, "col", 2, 1, &[Some("b"), Some("d")]);
    assert_eq!(e.array("chars").unwrap().class(), "char");
    strings(
        &e,
        "s",
        3,
        3,
        &[
            Some("a"),
            Some("changed"),
            None,
            Some("b"),
            Some("d"),
            None,
            None,
            None,
            Some("z"),
        ],
    );
    strings(
        &e,
        "alias",
        2,
        2,
        &[Some("a"), Some("c"), Some("b"), Some("d")],
    );
    strings(&e, "v", 1, 4, &[Some("a"), None, Some("d"), Some("e")]);
    e.execute("s(:,2)=[]; s(2,:)=[];").unwrap();
    strings(&e, "s", 2, 2, &[Some("a"), None, None, Some("z")]);
    e.execute("s(:)=[];").unwrap();
    strings(&e, "s", 0, 0, &[]);
    let e = run(r#"s=["a" "b"]; alias=s;"#);
    assert!(
        e.array("s")
            .unwrap()
            .same_snapshot(&e.array("alias").unwrap())
    );
}
#[test]
fn operators_missing_and_unicode_order() {
    let e = run(
        r#"a=["a";"b"]+[1 2]; b="x"+missing; c=["a" "β" missing]; eq=c==["a" "α" missing]; ne=c~=["a" "α" missing]; lt=c<"β"; le=c<="β"; gt=c>"a"; ge=c>="a"; m=ismissing(["" missing]);"#,
    );
    strings(
        &e,
        "a",
        2,
        2,
        &[Some("a1"), Some("b1"), Some("a2"), Some("b2")],
    );
    strings(&e, "b", 1, 1, &[None]);
    numeric(&e, "eq", &[1., 0., 0.]);
    numeric(&e, "ne", &[0., 1., 1.]);
    numeric(&e, "lt", &[1., 0., 0.]);
    numeric(&e, "le", &[1., 1., 0.]);
    numeric(&e, "gt", &[0., 1., 0.]);
    numeric(&e, "ge", &[1., 1., 0.]);
    numeric(&e, "m", &[0., 1.]);
}
#[test]
fn mapped_text_and_patterns() {
    let e = run(
        r#"u=upper(["ab" "Cd" missing]); l=lower(["AB";"Cd"]); t=strtrim([" a " " b "]); st=strip(["--a--" "--b--"],'left','-'); c=contains(["Abc" "def" missing],"B",'IgnoreCase',true); sw=startsWith(["Abc" "def"],"a",'IgnoreCase',true); ew=endsWith(["Abc" "def"],"F",'IgnoreCase',true); rep=replace(["ab" "ac"],"a","z"); sr=strrep(["ab" "aa"],"a","z"); er=erase(["ab" "ac"],"a"); rev=reverse(["a😀" "bc"]); p=pad(["a" "bc"],4,'both','-'); cat=strcat(["a " "b"],"!");"#,
    );
    strings(&e, "u", 1, 3, &[Some("AB"), Some("CD"), None]);
    strings(&e, "l", 2, 1, &[Some("ab"), Some("cd")]);
    strings(&e, "t", 1, 2, &[Some("a"), Some("b")]);
    strings(&e, "st", 1, 2, &[Some("a--"), Some("b--")]);
    numeric(&e, "c", &[1., 0., 0.]);
    numeric(&e, "sw", &[1., 0.]);
    numeric(&e, "ew", &[0., 1.]);
    strings(&e, "rep", 1, 2, &[Some("zb"), Some("zc")]);
    strings(&e, "sr", 1, 2, &[Some("zb"), Some("zz")]);
    strings(&e, "er", 1, 2, &[Some("b"), Some("c")]);
    strings(&e, "rev", 1, 2, &[Some("😀a"), Some("cb")]);
    strings(&e, "p", 1, 2, &[Some("-a--"), Some("-bc-")]);
    strings(&e, "cat", 1, 2, &[Some("a !"), Some("b!")]);
}
#[test]
fn splitting_joining_formatting_and_extracting() {
    let e = run(
        r#"a=split("a,b,c",","); c=split(["a:b";"c:d"],":"); j=join(["a" "b";"c" "d"],"-"); sj=strjoin(["a" "b"],","); f=sprintf("x=%d",3); co=compose("x=%d",[1 2]); before=extractBefore(["abc" "def"],2); after=extractAfter(["abc" "def"],1); mid=extractBetween(["abcd" "efgh"],2,3); bt=extractBetween("<abc>","<",">"); ib=insertBefore(["abc" "def"],2,"!"); ia=insertAfter(["abc" "def"],"e","!"); nl=newline;"#,
    );
    strings(&e, "a", 3, 1, &[Some("a"), Some("b"), Some("c")]);
    strings(&e, "c", 2, 2, &[Some("a"), Some("c"), Some("b"), Some("d")]);
    strings(&e, "j", 2, 1, &[Some("a-b"), Some("c-d")]);
    text(&e, "sj", "a,b");
    text(&e, "f", "x=3");
    strings(&e, "co", 1, 2, &[Some("x=1"), Some("x=2")]);
    strings(&e, "before", 1, 2, &[Some("a"), Some("d")]);
    strings(&e, "after", 1, 2, &[Some("bc"), Some("ef")]);
    strings(&e, "mid", 1, 2, &[Some("bc"), Some("fg")]);
    text(&e, "bt", "abc");
    strings(&e, "ib", 1, 2, &[Some("a!bc"), Some("d!ef")]);
    strings(&e, "ia", 1, 2, &[Some("abc"), Some("de!f")]);
    assert_eq!(e.array("nl").unwrap().class(), "char");
}
#[test]
fn regexp_outputs_and_containers() {
    let e = run(
        r#"s=["a1" "b2"]; r=regexprep(s,'[0-9]','X'); m=regexp("a1b2",'[a-z]','match'); [start,stop]=regexp("a1b2",'[a-z]'); ci=regexpi("Ab",'a','match'); cells={"ab",["x" "y"]}; a=cellfun(@strlength,cells,'UniformOutput',false); b=arrayfun(@strlength,s); obj=struct('label',"hello"); field=obj.label; c=arrayfun(@(x) x.label,obj,'UniformOutput',false);"#,
    );
    strings(&e, "r", 1, 2, &[Some("aX"), Some("bX")]);
    strings(&e, "m", 1, 2, &[Some("a"), Some("b")]);
    numeric(&e, "start", &[1., 3.]);
    numeric(&e, "stop", &[1., 3.]);
    text(&e, "ci", "A");
    text(&e, "field", "hello");
    numeric(&e, "b", &[2., 2.]);
    let Array::Cell(c) = e.array("cells").unwrap() else {
        panic!()
    };
    assert!(matches!(c.get_linear(0).unwrap().as_ref(), Array::Str(_)));
    let Array::Cell(c) = e.array("c").unwrap() else {
        panic!()
    };
    assert!(matches!(c.get_linear(0).unwrap().as_ref(), Array::Str(_)));
}
#[test]
fn for_loops_iterate_columns_and_scalar_options_survive_storage() {
    let e = run(
        r#"columns=cell(1,2); k=0; for s=["a" "b";"c" "d"]; k=k+1; columns{k}=s; end; first=columns{1}; second=columns{2}; joined=""; for s=["a" "b"]; joined=joined+s; end; opt="LineWidth"; plot(1:3,opt,2); title("hello"); count=0; for s=strings(0,2); count=count+1; end;"#,
    );
    strings(&e, "first", 2, 1, &[Some("a"), Some("c")]);
    strings(&e, "second", 2, 1, &[Some("b"), Some("d")]);
    text(&e, "joined", "ab");
    numeric(&e, "count", &[0.]);
}
#[test]
fn guards_limits_and_cancellation() {
    let mut e = run(r#"s=["a" "b"];"#);
    assert!(
        e.array("s")
            .unwrap()
            .to_numeric(&e.cancellation())
            .unwrap_err()
            .contains("string arrays")
    );
    for (code, fragment) in [
        (
            "inv(s)",
            "inv does not support string arrays yet; convert with char(...) or cellstr(...)",
        ),
        ("[a,b]=eig(s)", "eig does not support string arrays yet"),
        ("num2str(s)", "does not support string arrays yet"),
        (r#"num2str("a")"#, "does not accept string"),
        (r#"split(["a:b" "c"],':')"#, "same number"),
        ("strings(65537,1)", "65536"),
        ("strings(2,2,2)", "two dimensions"),
        ("s(65537)=\"x\";", "Index exceeds"),
        ("s(0)", "Subscript indices"),
    ] {
        let err = e.execute(code).unwrap_err();
        assert!(
            err.to_lowercase().contains(&fragment.to_lowercase()),
            "{code}: {err}"
        );
    }
    e.cancellation().store(true, Ordering::Relaxed);
    assert!(e.execute("upper(s)").unwrap_err().contains("cancelled"));
    assert!(
        StringArray::new(1, 1, vec![Some(Arc::from("x".repeat(2 * 1024 * 1024 + 1)))]).is_err()
    );
}
#[test]
fn display_workspace_and_read_only_values() {
    let mut e = Engine::default();
    for code in [
        r#"s="abc""#,
        r#"s=["a" "bb"]"#,
        r#"s=["a" "bb";"cc" missing]"#,
        "s=strings(0,0)",
        "s=missing",
    ] {
        let output = e.execute(code).unwrap().output.join("\n");
        assert!(output.contains("s ="), "{output}");
        if code.contains("0,0") {
            assert!(output.contains("0\u{00d7}0 empty string array"), "{output}");
        }
        if code.contains("missing") {
            assert!(output.contains("<missing>"), "{output}");
            assert!(!output.contains("\"<missing>\""));
        }
        if code.contains("abc") {
            assert!(output.contains("\"abc\""), "{output}");
        }
    }
    e.execute(r#"s=["a" "b"];"#).unwrap();
    let out = e.execute("whos").unwrap().output.join("\n");
    assert!(out.contains("string"), "{out}");
    // String arrays are editable in the spreadsheet Variable Editor (step ui-windows-clarity).
    assert!(matrust::variable_edit::Target::new("s".into(), e.array("s").unwrap(), vec![]).is_ok());
    let loose = e.execute("format loose;s=\"x\"").unwrap().output.join("\n");
    let compact = e
        .execute("format compact;s=\"x\"")
        .unwrap()
        .output
        .join("\n");
    assert!(loose.len() > compact.len());
}
#[test]
fn mat_saving_rejects_strings_atomically_including_empty_and_cells() {
    let dir = tempfile::tempdir().unwrap();
    for value in [r#""abc""#, "strings(0,0)", r#"{"a"}"#] {
        for option in ["-v6", "-v7"] {
            let path = dir.path().join("sentinel.mat");
            std::fs::write(&path, b"keep").unwrap();
            let mut e = run(&format!("s={value};"));
            let err = e
                .execute(&format!("save('{}','s','{option}');", path.display()))
                .unwrap_err();
            assert!(
                err.contains("string arrays cannot be saved to MAT files yet; use cellstr(...)"),
                "{err}"
            );
            assert_eq!(std::fs::read(&path).unwrap(), b"keep");
        }
        #[cfg(feature = "mat-v73")]
        {
            let path = dir.path().join("sentinel73.mat");
            std::fs::write(&path, b"keep").unwrap();
            let mut e = run(&format!("s={value};"));
            let err = e
                .execute(&format!("save('{}','s','-v7.3');", path.display()))
                .unwrap_err();
            assert!(
                err.contains("string arrays cannot be saved to MAT files yet; use cellstr(...)"),
                "{err}"
            );
            assert_eq!(std::fs::read(&path).unwrap(), b"keep");
        }
    }
}

#[test]
fn nested_updates_uniform_collection_and_empty_paths() {
    let e = run(
        r#"a=cellfun(@upper,{"ab","cd"}); b=arrayfun(@upper,["x";"y"]); c={struct('s',["a" "b"])}; c{1}.s(2)="z"; d=c{1}.s; sub=d{1}; blank=strings(0,2); q=upper(blank); eq=isequal({1},"a"); splitmissing=split(["a:b";missing],":"); j=join(["a";"b"],"-");"#,
    );
    strings(&e, "a", 1, 2, &[Some("AB"), Some("CD")]);
    strings(&e, "b", 2, 1, &[Some("X"), Some("Y")]);
    strings(&e, "d", 1, 2, &[Some("a"), Some("z")]);
    strings(&e, "q", 0, 2, &[]);
    numeric(&e, "eq", &[0.]);
    strings(
        &e,
        "splitmissing",
        2,
        2,
        &[Some("a"), None, Some("b"), None],
    );
    text(&e, "j", "a-b");
    let mut e = Engine::default();
    for code in [
        r#"strip(strings(0,0),'bad')"#,
        r#"pad(strings(0,0),-1)"#,
        r#"replace("a","b","c",1)"#,
        r#"missing*missing"#,
    ] {
        assert!(e.execute(code).is_err(), "{code}");
    }
}

#[test]
fn output_growth_is_bounded_and_failed_assignment_is_atomic() {
    let mut e = run(r#"s="keep";"#);
    for code in [
        r#"s=pad(strings(1,100),65536);"#,
        r#"s=string(repmat({repmat('a',1,65536)},1,33));"#,
        r#"s=compose("%65536s",strings(1,100));"#,
        r#"s=split(pad("",65537,'right',':'),':','CollapseDelimiters',false);"#,
    ] {
        assert!(e.execute(code).is_err(), "{code}");
        text(&e, "s", "keep");
    }
}

#[test]
fn regex_array_patterns_tokens_and_missing_outputs() {
    let e = run(
        r#"m=regexp(["ab" "12"],["[a-z]" "[0-9]"],'match'); a=m{1}; b=m{2}; t=regexp("ab12",'([a-z]+)([0-9]+)','tokens'); tok=t{1}; miss=regexp(missing,'a','match'); extracted=extractBetween("x>y<z>","<",">");"#,
    );
    strings(&e, "a", 1, 2, &[Some("a"), Some("b")]);
    strings(&e, "b", 1, 2, &[Some("1"), Some("2")]);
    strings(&e, "tok", 1, 2, &[Some("ab"), Some("12")]);
    strings(&e, "miss", 1, 1, &[None]);
    text(&e, "extracted", "z");
}

#[test]
fn empty_and_multiple_brace_contents_do_not_panic_in_expressions() {
    let mut e = run(r#"s=["a" "b"];"#);
    for code in ["x=s{[]}+1;", "x=s{1:2}+1;"] {
        assert!(e.execute(code).is_err(), "{code}");
    }
    e.execute("[a,b]=s{1:2};").unwrap();
    assert_eq!(e.array("a").unwrap().preview(1), "'a'");
    assert_eq!(e.array("b").unwrap().preview(1), "'b'");
}

#[test]
fn missing_iteration_typed_positions_and_empty_split() {
    let e = run(
        r#"cells=cell(1,2); n=0; for s=[missing "x"]; n=n+1; cells{n}=s; end; m=cells{1}; x=cells{2}; a=extractBefore("abcd",int32(3)); b=insertAfter("abcd",int32(2),"!"); empty=split(""); ext=regexp("",'a','tokenextents');"#,
    );
    strings(&e, "m", 1, 1, &[None]);
    text(&e, "x", "x");
    text(&e, "a", "ab");
    text(&e, "b", "ab!cd");
    text(&e, "empty", "");
    assert_eq!(e.array("ext").unwrap().class(), "cell");
}

#[test]
fn scalar_aliases_and_workspace_inspection_share_string_storage() {
    let mut e = run(r#"s=pad("a",10000); alias=s; typed=int32(3)+"x";"#);
    let original = e.array("s").unwrap();
    assert!(original.same_snapshot(&e.array("alias").unwrap()));
    assert!(original.same_snapshot(&e.workspace()["s"]));
    e.rename_array("alias", "renamed").unwrap();
    assert!(original.same_snapshot(&e.array("renamed").unwrap()));
    e.execute(r#"s(1)="changed";"#).unwrap();
    assert!(original.same_snapshot(&e.array("renamed").unwrap()));
    text(&e, "typed", "3x");
}

#[test]
fn complex_numeric_string_conversion_reuses_the_existing_parser() {
    let e = run(r#"s=string([1+2i 3-4i]); n=str2double(s); d=double(["2i" "bad"]);"#);
    strings(&e, "s", 1, 2, &[Some("1+2i"), Some("3-4i")]);
    let Array::Complex(n) = e.array("n").unwrap() else {
        panic!("complex conversion")
    };
    assert_eq!(
        n.data(),
        &[
            matrust::Complex64::new(1., 2.),
            matrust::Complex64::new(3., -4.)
        ]
    );
    let Array::Complex(d) = e.array("d").unwrap() else {
        panic!("complex conversion")
    };
    assert_eq!(d.data()[0], matrust::Complex64::new(0., 2.));
    assert!(d.data()[1].re.is_nan());
}

#[test]
fn string_guard_only_restricts_builtins_not_user_functions() {
    let e = run(
        r#"s=["a" "b"]; pass(s); standalone=ans; one=pass(s); [first,second]=parts(s);
function y=pass(x); y=upper(x); end
function [a,b]=parts(x); a=x(1); b=x(2); end"#,
    );
    strings(&e, "standalone", 1, 2, &[Some("A"), Some("B")]);
    strings(&e, "one", 1, 2, &[Some("A"), Some("B")]);
    text(&e, "first", "a");
    text(&e, "second", "b");
    let e = run(r#"s=["a" "b"]; x=inv(s); function y=inv(x); y=upper(x); end"#);
    strings(&e, "x", 1, 2, &[Some("A"), Some("B")]);
}

#[test]
fn round6_03_disp_retains_array_quotes_indent_and_alignment() {
    let mut e = Engine::default();
    for compact in ["compact", "loose"] {
        e.execute(&format!("format {compact};")).unwrap();
        assert_eq!(e.execute(r#"disp("abc");"#).unwrap().output, ["abc"]);
        assert_eq!(
            e.execute(r#"disp(["a" "b";"long" missing]);"#)
                .unwrap()
                .output,
            ["    \"a\"       \"b\"\n    \"long\"    <missing>"]
        );
    }
}

#[test]
fn round6_04_extract_between_returns_all_text_boundary_pairs() {
    let e = run(r#"a=extractBetween("<a><b>","<",">");
        b=extractBetween("x<a>middle<b>tail<unfinished","<",">");
        c=extractBetween('<a><b>','<','>');
        d=extractBetween(["<a><b>";"<c><d>"],"<",">");
        empty=extractBetween("none","<",">");
        unicode=extractBetween("😀<α><β>","<",">");
        positions=extractBetween("abcd",2,3);"#);
    strings(&e, "a", 2, 1, &[Some("a"), Some("b")]);
    strings(&e, "b", 2, 1, &[Some("a"), Some("b")]);
    chars(&e, "c", "ab");
    assert_eq!(e.array("c").unwrap().dims().as_ref(), &[2, 1]);
    strings(&e, "d", 2, 2, &[Some("a"), Some("c"), Some("b"), Some("d")]);
    strings(&e, "empty", 0, 1, &[]);
    strings(&e, "unicode", 2, 1, &[Some("α"), Some("β")]);
    text(&e, "positions", "bc");
    assert!(
        Engine::default()
            .execute(r#"extractBetween(["<a>";"<b><c>"],"<",">")"#)
            .unwrap_err()
            .contains("same number")
    );
    assert!(
        Engine::default()
            .execute(r#"extractBetween(["<a><b>" "<c><d>"],"<",">")"#)
            .unwrap_err()
            .contains("N-D")
    );
}

#[test]
fn round6_05_compose_consumes_data_columns_across_each_row() {
    let e = run(r#"a=compose("%d %d",[1 2]);
        b=compose("%d-%d",[1 2;3 4]);
        c=compose('%d-%d',int8([1 2;3 4]));
        d=compose("%d-%d",[1 2 5 6;3 4 7 8]);
        escaped=compose("%% %d-%d",[1 2]);
        one=compose("%d",[1 2;3 4]);
        stars=compose("%*d",[3 7]);
        texts=compose("%s:%s",["a" "b";"c" missing]);"#);
    text(&e, "a", "1 2");
    strings(&e, "b", 2, 1, &[Some("1-2"), Some("3-4")]);
    cellstr(&e, "c", (2, 1), &["1-2", "3-4"]);
    strings(
        &e,
        "d",
        2,
        2,
        &[Some("1-2"), Some("3-4"), Some("5-6"), Some("7-8")],
    );
    text(&e, "escaped", "% 1-2");
    strings(
        &e,
        "one",
        2,
        2,
        &[Some("1"), Some("3"), Some("2"), Some("4")],
    );
    text(&e, "stars", "  7");
    strings(&e, "texts", 2, 1, &[Some("a:b"), None]);
    assert!(
        Engine::default()
            .execute(r#"compose("%d %d",[1 2 3])"#)
            .unwrap_err()
            .contains("multiple")
    );
}

/// MathWorks strtrim and strip (Algorithms): whitespace is removed, except the
/// significant whitespace characters char(133), char(160), char(8199), char(8239).
/// char(8192) (en quad) and char(12288) (ideographic space) are ordinary
/// Unicode whitespace and are removed. Naming char(160) as the strip character
/// still removes it.
#[test]
fn review_1111_strtrim_and_strip_keep_significant_whitespace() {
    let e = run(r#"kept=strtrim(string(char([32 160 97 133 32])));
        kept2=strtrim(string(char([8199 98 8239])));
        gone=strtrim(string(char([8192 99 12288])));
        sk=strip(string(char([32 160 97 160 32])));
        sg=strip(string(char([8192 99 8192])));
        side=strip(string(char([32 160 97 32])),'right');
        named=strip(string(char([160 97 160])),char(160));"#);
    text(&e, "kept", "\u{00A0}a\u{0085}");
    text(&e, "kept2", "\u{2007}b\u{202F}");
    text(&e, "gone", "c");
    text(&e, "sk", "\u{00A0}a\u{00A0}");
    text(&e, "sg", "c");
    text(&e, "side", " \u{00A0}a");
    text(&e, "named", "a");
}

/// MathWorks cellstr: one cell per row. cellstr('') is {''}.
/// char(zeros(0,3)) has no rows, so the cell is 0-by-1.
/// char(zeros(2,0)) has two empty rows, so the cell is 2-by-1 of ''.
#[test]
fn review_1111_cellstr_preserves_empty_row_counts() {
    let e = run(
        r#"z=cellstr(''); wide=cellstr(char(zeros(0,3))); tall=cellstr(char(zeros(2,0)));
        zs=size(z); ws=size(wide); ts=size(tall); sa=size(char(zeros(0,3))); sb=size(char(zeros(2,0)));"#,
    );
    numeric(&e, "sa", &[0., 3.]);
    numeric(&e, "sb", &[2., 0.]);
    numeric(&e, "zs", &[1., 1.]);
    numeric(&e, "ws", &[0., 1.]);
    numeric(&e, "ts", &[2., 1.]);
    cellstr(&e, "z", (1, 1), &[""]);
    cellstr(&e, "wide", (0, 1), &[]);
    cellstr(&e, "tall", (2, 1), &["", ""]);
    let Array::Cell(cells) = e.array("tall").unwrap() else {
        panic!("tall");
    };
    let Array::Character(row) = cells.get_linear(0).unwrap().as_ref() else {
        panic!("tall element");
    };
    assert_eq!((row.rows(), row.cols()), (0, 0));
}

/// MathWorks pad: the pad character must be a single character. Width counts
/// UTF-16 units, so U+1F600 (two units) is not one character. A one-unit
/// character still pads: pad("a",3,"-") is "a--" with strlength 3.
#[test]
fn review_1111_pad_requires_one_utf16_unit() {
    let err = Engine::default()
        .execute(r#"p = pad("a",3,"😀");"#)
        .unwrap_err();
    assert!(
        err.contains("pad character must be a single character"),
        "{err}"
    );
    let err = Engine::default()
        .execute(r#"p = pad("a",3,"both","😀");"#)
        .unwrap_err();
    assert!(
        err.contains("pad character must be a single character"),
        "{err}"
    );
    let e = run(r#"p=pad("a",3,"-"); n=strlength(p); q=pad("é",3); m=strlength(q);"#);
    text(&e, "p", "a--");
    numeric(&e, "n", &[3.]);
    text(&e, "q", "é  ");
    numeric(&e, "m", &[3.]);
}

#[test]
fn fix14_sprintf_and_fprintf_string_arrays() {
    let e = run(r#"
        a = sprintf('%s,', ["a", "b"]);
        b = sprintf("%s,", ["a", "b"]);
        c = sprintf('%s ', ["a" "c"; "b" "d"]);
        d = sprintf('%s %s %d %d', ["x", "y"], [10, 20]);
        fmt = "%s|";
        f = sprintf(fmt, ["p", "q"]);
        "#);
    chars(&e, "a", "a,b,");
    strings(&e, "b", 1, 1, &[Some("a,b,")]);
    chars(&e, "c", "a b c d ");
    chars(&e, "d", "x y 10 20");
    strings(&e, "f", 1, 1, &[Some("p|q|")]);

    let mut e2 = Engine::default();
    let out1 = e2.execute(r#"fprintf('%s\n', ["a", "b"])"#).unwrap();
    assert_eq!(out1.output, vec!["a", "b"]);

    let out2 = e2.execute(r#"n = fprintf("%s\n", ["a", "b"]);"#).unwrap();
    assert_eq!(out2.output, vec!["a", "b"]);
    numeric(&e2, "n", &[4.]);

    let out3 = e2
        .execute(r#"s = ["u", "v"]; fprintf('%s,\n', s);"#)
        .unwrap();
    assert_eq!(out3.output, vec!["u,", "v,"]);
}

#[test]
fn fix14_sprintf_rejects_too_many_empty_string_arguments_before_allocating_items() {
    let mut e = run("x = strings(1, 65536);");
    // Reuse one valid array to exceed the 8 MiB argument limit with empty strings.
    let call = format!("s = sprintf('%s'{});", ", x".repeat(129));
    let error = e.execute(&call).unwrap_err();
    assert!(error.contains("sprintf: too many arguments"), "{error}");
    assert!(e.array("s").is_none());
}
