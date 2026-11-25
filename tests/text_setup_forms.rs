use matrust::Engine;
use std::fs;

#[test]
fn regexp_treats_only_unmatched_parentheses_as_literals() {
    let mut engine = Engine::default();
    // iFEM's corpus script tool/myunique.m uses regexp(version,'(') under MATLAB.
    // Octave errors on that pattern.
    engine
        .execute(
            r#"
assert(regexp('MatRust (R2024b)', '(') == 9);
assert(isequal(regexpi('xA(X)a(', 'a('), [2 6]));
assert(strcmp(regexprep('x) y)', ')', '!'), 'x! y!'));
assert(strcmp(regexprep('x(ab', '((ab)', '$1!'), 'xab!'));
r = regexp(')ab(', ')(?<part>ab)(', 'names');
assert(strcmp(r.part, 'ab'));
assert(isequal(regexp('a(b)c', '[()]'), [2 4]));
assert(regexp('a(b)c', '\(') == 2);
assert(regexp('ab', '(?# ignored ()(ab)') == 1);
assert(regexp('ab(', sprintf('(ab) ( # ignored )\n'), 'freespacing') == 1);
assert(strcmp(regexprep('ab', '(a)(b)', '$2$1'), 'ba'));
"#,
        )
        .unwrap();
    assert!(engine.execute("regexp('x','[');").is_err());
    assert!(engine.execute("regexp('x','(?=x)');").is_err());
}

#[test]
fn strcat_combines_character_matrices_row_by_row() {
    let mut engine = Engine::default();
    engine
        .execute(
            r#"
assert(isequal(strcat(repmat('p-',2,1), {'a';'b'}), {'p-a';'p-b'}));
assert(isequal(strcat(['ab';'cd'], 'x'), ['abx';'cdx']));
assert(isequal(strcat(['a ';'bb'], 'x '), ['ax ';'bbx']));
assert(isequal(strcat('p- ', ['a';'b'], ['c';'d']), ['p-ac';'p-bd']));
assert(isequal(strcat(['a ';'bb'], {'x '}), {'ax ';'bbx '}));
assert(isequal(strcat(['a ';'bb'], ["x ";"y"]), ["ax ";"bby"]));
assert(isequal(strcat(repmat('z',2,0), "x"), ["x";"x"]));
assert(isequal(strcat(['😀';'éx'], '!'), ['😀!';'éx!']));
assert(isequal(size(strcat(repmat(' ',2,1))), [2 0]));
"#,
        )
        .unwrap();
    for call in [
        "strcat(['a';'b'], {'x','y'});",
        "strcat(['a';'b'], ['x';'y';'z']);",
    ] {
        assert!(engine.execute(call).is_err(), "{call}");
    }
}

#[test]
fn strcat_ignores_zero_row_char_shape_in_mixed_cell_calls() {
    let mut engine = Engine::default();
    engine
        .execute(
            r#"
p = repmat('prior-', 0, 1);
cases = {{}, cell(0,1), cell(1,0), cell(2,0), {'x';'y'}, {'x' 'y'}};
for k = 1:numel(cases)
    c = cases{k};
    a = strcat(p, c);
    b = strcat(c, p);
    assert(iscell(a) && isequal(size(a), size(c)) && isequal(a, c));
    assert(iscell(b) && isequal(size(b), size(c)) && isequal(b, c));
end
assert(strcmp(a{1}, 'x') && strcmp(a{2}, 'y'));
q = [{'x'}; strcat(p, {})];
assert(iscell(q) && isequal(size(q), [1 1]) && isequal(q, {'x'}));
assert(isequal(strcat(repmat('prior-', 2, 1), {'a'; 'b'}), {'prior-a'; 'prior-b'}));
assert(ischar(strcat(p)) && isempty(strcat(p)));
assert(ischar(strcat(p, 'x')) && isempty(strcat(p, 'x')));
"#,
        )
        .unwrap();
    for call in [
        "p = repmat('prior-', 0, 1); strcat(p, {}, cell(0,1));",
        "p = repmat('prior-', 0, 1); strcat(p, cell(0,1), {});",
        "p = repmat('prior-', 0, 1); strcat({}, p, cell(0,1));",
        "p = repmat('prior-', 0, 1); strcat({}, cell(0,1), p);",
        "p = repmat('prior-', 0, 1); strcat(cell(0,1), p, {});",
        "p = repmat('prior-', 0, 1); strcat(cell(0,1), {}, p);",
        "strcat(repmat('prior-', 2, 1), {'a'; 'b'; 'c'});",
        "strcat(repmat('prior-', 2, 1), {});",
        "strcat({}, repmat('prior-', 2, 1));",
        "strcat(repmat('prior-', 2, 1), cell(0, 1));",
        "strcat(repmat('prior-', 2, 1), {'a', 'b'});",
    ] {
        assert!(engine.execute(call).is_err(), "{call}");
    }
}

#[test]
fn rng_generator_names_survive_seeding_and_state_restore() {
    let mut engine = Engine::default();
    for (name, kernel) in [
        ("mt19937ar", "twister"),
        ("twister", "twister"),
        ("simdTwister", "twister"),
        ("combRecursive", "combRecursive"),
        ("multFibonacci", "combRecursive"),
        ("philox", "philox"),
        ("threefry", "philox"),
        ("v4", "combRecursive"),
        ("v5uniform", "combRecursive"),
        ("v5normal", "twister"),
        ("SiMdTwIsTeR", "twister"),
    ] {
        let code = format!(
            r#"
rng(17, '{kernel}'); expected = rand(1,4);
rng(17, "{name}"); s = rng;
assert(strcmp(s.Type, '{name}') && s.Seed == 17 && isequal(rand(1,4), expected));
s = rng; sample = rand(1,4); rng('default'); rng(s); restored = rng;
assert(isequal(s, restored) && isequal(rand(1,4), sample));
rng('default', '{name}'); s = rng; sample = rand(1,4);
assert(strcmp(s.Type, '{name}') && s.Seed == 0);
rng(0, '{kernel}'); assert(isequal(rand(1,4), sample));
rng('shuffle', '{name}'); s = rng;
assert(strcmp(s.Type, '{name}') && isa(s.Seed, 'uint32'));
rng(17); s = rng;
assert(strcmp(s.Type, '{name}') && s.Seed == 17 && isequal(rand(1,4), expected));
rng('{name}'); s = rng;
assert(strcmp(s.Type, '{name}') && s.Seed == 0);
"#,
        );
        engine
            .execute(&code)
            .unwrap_or_else(|err| panic!("{name}: {err}"));
    }
}

#[test]
fn scripts_and_functions_fall_back_to_windows_1252() {
    let dir = tempfile::tempdir().unwrap();
    let legacy = b"% caf\xe9 \x93quote\r\nlegacy = 'caf\xe9'; smart = '\x93';\r\n";
    let path = dir.path().join("legacy_file.m");
    fs::write(&path, legacy).unwrap();
    assert_eq!(
        matrust::files::read_script(&path).unwrap(),
        "% café “quote\r\nlegacy = 'café'; smart = '“';\r\n"
    );
    fs::write(
        dir.path().join("legacy_function.m"),
        b"function s = legacy_function()\n% caf\xe9 \x93quote\ns = 'caf\xe9 \x93';\nend\n",
    )
    .unwrap();
    fs::write(dir.path().join("plain_utf8.m"), "plain = 'café “';\n").unwrap();
    fs::write(dir.path().join("bom_utf8.m"), "\u{feff}bom = 'café “';\n").unwrap();

    let mut engine = Engine::default();
    engine.set_function_directory(dir.path()).unwrap();
    engine.set_file_directory(dir.path()).unwrap();
    engine
        .execute(
            "run('legacy_file.m'); assert(strcmp(legacy, 'café') && double(smart) == 8220); \
             clear legacy; legacy_file; assert(strcmp(legacy, 'café')); \
             assert(strcmp(legacy_function(), 'café “')); \
             run('plain_utf8.m'); run('bom_utf8.m'); \
             assert(strcmp(plain, 'café “') && strcmp(bom, plain));",
        )
        .unwrap();
}

#[test]
fn script_source_decoder_rejects_nul_before_decoding() {
    use matrust::files::{ScriptDecodeError, decode_script_source};
    use std::path::Path;
    let decode = |name: &str, bytes: &[u8]| decode_script_source(Path::new(name), bytes.to_vec());
    let names = ["a.m", "a.M", "a.mr", "a.txt"];

    // Unchanged: valid UTF-8 for every extension; a UTF-8 BOM is kept for the
    // caller (read_script and the engine strip it).
    for name in names {
        let text = "x = 'café “';\r\n";
        assert_eq!(decode(name, text.as_bytes()).unwrap(), text, "{name}");
        assert_eq!(decode(name, b"").unwrap(), "", "{name}");
        assert_eq!(
            decode(name, b"\xef\xbb\xbfx = 1;\n").unwrap(),
            "\u{feff}x = 1;\n",
            "{name}"
        );
    }
    // Unchanged: Windows-1252 for .m files only; elsewhere the UTF-8 error
    // with the text callers have always shown.
    let legacy = b"s = 'caf\xe9 \x93\x80';\r\n";
    for name in ["a.m", "a.M"] {
        assert_eq!(
            decode(name, legacy).unwrap(),
            "s = 'café “€';\r\n",
            "{name}"
        );
    }
    for name in ["a.mr", "a.txt"] {
        let error = decode(name, legacy).unwrap_err();
        assert!(
            matches!(error, ScriptDecodeError::InvalidUtf8(_)),
            "{name}: {error}"
        );
        assert_eq!(
            error.to_string(),
            String::from_utf8(legacy.to_vec()).unwrap_err().to_string()
        );
    }

    // NUL is valid UTF-8 but never source text, whatever the extension:
    // UTF-16 `%a` with and without a BOM (review finding 6), a UTF-8 BOM
    // followed by NUL, a stray NUL, NUL next to Windows-1252 bytes.
    let nul: [&[u8]; 9] = [
        b"%\0a\0",
        b"\0%\0a",
        b"\xef\xbb\xbf\0",
        b"\xef\xbb\xbfx = 1;\0",
        b"\xff\xfe%\0a\0",
        b"\xfe\xff\0%\0a",
        b"x = 1;\0\n",
        b"\0",
        b"s = 'caf\xe9';\0",
    ];
    for bytes in nul {
        for name in names {
            assert_eq!(
                decode(name, bytes),
                Err(ScriptDecodeError::Nul),
                "{name} {bytes:?}"
            );
        }
    }
    // A UTF-16 byte-order mark without any NUL is never Windows-1252 text.
    let marked: [&[u8]; 4] = [
        b"\xff\xfe",
        b"\xfe\xff",
        b"\xff\xfe\x01\x01",
        b"\xfe\xffx = 1;\n",
    ];
    for bytes in marked {
        for name in names {
            assert_eq!(
                decode(name, bytes),
                Err(ScriptDecodeError::Utf16Bom),
                "{name} {bytes:?}"
            );
        }
    }
    assert_eq!(
        ScriptDecodeError::Nul.to_string(),
        "contains NUL bytes (UTF-16 or binary file?)"
    );
    assert!(ScriptDecodeError::Utf16Bom.to_string().contains("UTF-16"));
}

#[test]
fn nul_bearing_source_files_are_rejected_and_normal_ones_load() {
    let dir = tempfile::tempdir().unwrap();
    let wide = |text: &str| -> Vec<u8> { text.encode_utf16().flat_map(u16::to_le_bytes).collect() };
    fs::write(dir.path().join("wide_script.m"), b"%\0a\0").unwrap();
    fs::write(
        dir.path().join("wide_function.m"),
        wide("function y = wide_function()\ny = 1;\nend\n"),
    )
    .unwrap();
    fs::write(dir.path().join("nul_tail.mr"), b"tail = 1;\0\n").unwrap();
    fs::write(dir.path().join("marked.m"), b"\xff\xfe\x01\x01").unwrap();
    fs::write(dir.path().join("legacy.mr"), b"s = 'caf\xe9';\n").unwrap();
    fs::write(dir.path().join("plain_script.m"), "plain = 'café';\n").unwrap();
    fs::write(
        dir.path().join("plain_function.mr"),
        "function y = plain_function()\ny = 2;\nend\n",
    )
    .unwrap();

    for name in ["wide_script.m", "wide_function.m", "nul_tail.mr"] {
        assert_eq!(
            matrust::files::read_script(&dir.path().join(name)).unwrap_err(),
            "Script contains NUL bytes (UTF-16 or binary file?)",
            "{name}"
        );
    }
    // Unchanged messages and results.
    for name in ["marked.m", "legacy.mr"] {
        assert_eq!(
            matrust::files::read_script(&dir.path().join(name)).unwrap_err(),
            "Script is not valid UTF-8",
            "{name}"
        );
    }
    assert_eq!(
        matrust::files::read_script(&dir.path().join("plain_script.m")).unwrap(),
        "plain = 'café';\n"
    );

    let mut engine = Engine::default();
    engine.set_function_directory(dir.path()).unwrap();
    engine.set_file_directory(dir.path()).unwrap();
    for call in [
        "run('wide_script.m')",
        "wide_script",
        "y = wide_function();",
        "run('nul_tail.mr')",
        "nul_tail",
    ] {
        let error = engine.execute(call).unwrap_err();
        assert!(
            error.contains("contains NUL bytes (UTF-16 or binary file?)"),
            "{call}: {error}"
        );
    }
    engine
        .execute(
            "assert(exist('tail', 'var') == 0 && exist('y', 'var') == 0); \
             run('plain_script.m'); assert(strcmp(plain, 'café')); \
             clear plain; plain_script; assert(strcmp(plain, 'café')); \
             assert(plain_function() == 2);",
        )
        .unwrap();
}
