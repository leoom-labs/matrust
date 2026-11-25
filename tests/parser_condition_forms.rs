use matrust::{Engine, syntax};

fn check_script_and_line(lines: &[&str], check: impl Fn(&Engine, &[String])) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("condition_forms.m"), lines.join("\n")).unwrap();
    for source in ["condition_forms".to_owned(), lines.join(" ")] {
        let mut engine = Engine::default();
        engine.set_file_directory(dir.path()).unwrap();
        let result = engine
            .execute(&source)
            .unwrap_or_else(|err| panic!("{source}: {err}"));
        check(&engine, &result.output);
    }
}

fn assert_scalar(engine: &Engine, name: &str, expected: f64) {
    assert_eq!(engine.variables[name].data(), &[expected], "{name}");
}

#[test]
fn parenthesized_conditions_allow_inline_statements() {
    check_script_and_line(
        &[
            "tau = 3; hits = 0;",
            "for t = 1:5,",
            "    if (t>=tau) break; end;",
            "    hits = hits+1;",
            "end;",
            "i = 3;",
            "if (i>2) a = 1; end;",
            "k = 0;",
            "while (k<3) k = k+1; end;",
            "x = 1;",
            "if (false) y = 0; elseif (x) y = 2; else y = 3; end;",
            "if (a) disp(1), end;",
        ],
        |engine, output| {
            assert_scalar(engine, "hits", 2.);
            assert_scalar(engine, "t", 3.);
            assert_scalar(engine, "a", 1.);
            assert_scalar(engine, "k", 3.);
            assert_scalar(engine, "y", 2.);
            assert_eq!(output.iter().map(|s| s.trim()).collect::<Vec<_>>(), ["1"]);
        },
    );
}

#[test]
fn conditions_consume_expression_continuations_before_starting_the_body() {
    check_script_and_line(
        &[
            "a = -1; b = 0; c = 0;",
            "if (a)+1 > 0, b = 1; end;",
            "if (a)+1 == (0) c = 2; end;",
            "if ((a) < 0) if (c) d = 3; end; end;",
            "if (c) disp inline; end;",
            "if (c) local = 4; end;",
            "local -1; e = ans;",
        ],
        |engine, output| {
            assert_scalar(engine, "b", 0.);
            assert_scalar(engine, "c", 2.);
            assert_scalar(engine, "d", 3.);
            assert_scalar(engine, "e", 3.);
            assert!(output.iter().any(|s| s.contains("inline")), "{output:?}");
        },
    );
    for source in [
        "if x y = 1; end",
        "if (a)+1 > 0 y = 1; end",
        "if (a) end",
        "if (a), elseif x y = 1; end",
        "while x y = 1; end",
        "for k = (1:2) y = 1; end",
        "switch (x) case 1, y = 1; end",
    ] {
        assert!(syntax::parse(source).is_err(), "{source}");
    }
    assert!(syntax::is_incomplete("if (a) disp(1)"));
    assert!(!syntax::is_incomplete("if (a) disp(1), end"));
}

#[test]
fn clear_command_mixes_k_wave_glob_and_exact_names() {
    check_script_and_line(
        &[
            "a_DEF = 1; b_DEF = 2; NAME = 3; other = 4; keep = 5;",
            "clear *_DEF NAME other;",
            "after = keep;",
        ],
        |engine, _| {
            for name in ["a_DEF", "b_DEF", "NAME", "other"] {
                assert!(!engine.contains_array(name), "{name}");
            }
            assert_scalar(engine, "keep", 5.);
            assert_scalar(engine, "after", 5.);
        },
    );
}

#[test]
fn clear_and_clearvars_globs_match_entire_case_sensitive_names() {
    for command in [
        "clear a* ?_DEF plain",
        "clear('a*','?_DEF','plain')",
        "clearvars ?_DEF a* plain",
        "clearvars('a*','?_DEF','plain')",
    ] {
        check_script_and_line(
            &[
                "a = 1; aLong = 2; A = 3; plain = 4;",
                "b_DEF = true; c_DEF = 1i; d_DEF = 'text'; bb_DEF = 5;",
                &format!("{command};"),
            ],
            |engine, _| {
                for name in ["a", "aLong", "b_DEF", "c_DEF", "d_DEF", "plain"] {
                    assert!(!engine.contains_array(name), "{command}: {name}");
                }
                assert_scalar(engine, "A", 3.);
                assert_scalar(engine, "bb_DEF", 5.);
            },
        );
    }
}

#[test]
fn clear_inside_inline_body_and_clearvars_options() {
    check_script_and_line(
        &[
            "a1 = 1; a2 = 2; b1 = 3; b2 = 4; c1 = 5; keep = 6;",
            "if (keep) clear b1; end;",
            "clearvars a* -except a2;",
            "clearvars -regexp '^b[2-9]$';",
            "clearvars -except a1 a2 b1 b2 keep;",
        ],
        |engine, _| {
            assert_eq!(engine.workspace_count(), 2);
            assert_scalar(engine, "a2", 2.);
            assert_scalar(engine, "keep", 6.);
        },
    );
}

#[test]
fn clear_single_glob_and_invalid_mixed_list() {
    let mut engine = Engine::default();
    engine.execute("a = 1; aLong = 2; keep = 3;").unwrap();
    assert!(engine.execute("clear a* 'bad-name';").is_err());
    assert_scalar(&engine, "a", 1.);
    assert_scalar(&engine, "aLong", 2.);
    engine.execute("clear('a*');").unwrap();
    assert_eq!(engine.workspace_count(), 1);
    assert_scalar(&engine, "keep", 3.);
    engine.execute("clear *;").unwrap();
    assert_eq!(engine.workspace_count(), 0);
}
