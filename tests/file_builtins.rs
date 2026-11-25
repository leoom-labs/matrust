use matrust::{Array, Engine};
use std::fs;
use std::process::Command;

fn scalar(engine: &Engine, name: &str) -> f64 {
    engine.variables[name].data()[0]
}

fn string_col(engine: &Engine, name: &str) -> Vec<String> {
    match engine.array(name).unwrap() {
        Array::Str(sa) => {
            assert_eq!(sa.cols(), 1, "{name} should be a column string array");
            sa.data()
                .iter()
                .map(|s| s.as_ref().expect("non-missing").to_string())
                .collect()
        }
        other => panic!("expected string array for {name}, got {other:?}"),
    }
}

fn char_text(engine: &Engine, name: &str) -> String {
    match engine.array(name).unwrap() {
        Array::Character(c) => c.to_utf8(&engine.cancellation()).unwrap(),
        other => panic!("expected char array for {name}, got {other:?}"),
    }
}

#[test]
fn readlines_and_writelines_round_trip_and_options() {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = Engine::default();
    engine.set_current_folder(dir.path()).unwrap();
    engine.set_function_directory(dir.path()).unwrap();

    // Default writelines uses "\n" on all platforms, and readlines does not add trailing empty line.
    engine
        .execute(
            r#"
            writelines(["alpha"; "beta"; "gamma"], 'rt.txt');
            lines = readlines('rt.txt');
            "#,
        )
        .unwrap();
    assert_eq!(string_col(&engine, "lines"), vec!["alpha", "beta", "gamma"]);
    assert_eq!(
        fs::read(dir.path().join("rt.txt")).unwrap(),
        b"alpha\nbeta\ngamma\n"
    );

    // Append mode, char array (multi-row), and cellstr.
    engine
        .execute(
            r#"
            writelines(['de'; 'fg'], 'rt.txt', 'WriteMode', 'append');
            writelines({'hi', 'jk'}, 'rt.txt', 'WriteMode', 'append');
            lines2 = readlines('rt.txt');
            "#,
        )
        .unwrap();
    assert_eq!(
        string_col(&engine, "lines2"),
        vec!["alpha", "beta", "gamma", "de", "fg", "hi", "jk"]
    );

    // Empty file and single empty line round-trip.
    fs::write(dir.path().join("empty.txt"), b"").unwrap();
    engine.execute("e = readlines('empty.txt');").unwrap();
    assert_eq!(string_col(&engine, "e"), Vec::<String>::new());

    engine
        .execute(
            r#"
            writelines("", 'one_empty.txt');
            oe = readlines('one_empty.txt');
            "#,
        )
        .unwrap();
    assert_eq!(string_col(&engine, "oe"), vec![""]);

    // WhitespaceRule and EmptyLineRule with mixed line endings.
    fs::write(
        dir.path().join("mixed.txt"),
        b"  lead  \r\n\n\ttrail\t\r  both  \r\n",
    )
    .unwrap();
    engine
        .execute(
            r#"
            w_pres = readlines('mixed.txt', 'WhitespaceRule', 'preserve');
            w_trim = readlines('mixed.txt', 'WhitespaceRule', 'trim');
            w_lead = readlines('mixed.txt', 'WhitespaceRule', 'trimleading');
            w_trail = readlines('mixed.txt', 'WhitespaceRule', 'trimtrailing');
            e_skip = readlines('mixed.txt', 'EmptyLineRule', 'skip');
            "#,
        )
        .unwrap();
    assert_eq!(
        string_col(&engine, "w_pres"),
        vec!["  lead  ", "", "\ttrail\t", "  both  "]
    );
    assert_eq!(
        string_col(&engine, "w_trim"),
        vec!["lead", "", "trail", "both"]
    );
    assert_eq!(
        string_col(&engine, "w_lead"),
        vec!["lead  ", "", "trail\t", "both  "]
    );
    assert_eq!(
        string_col(&engine, "w_trail"),
        vec!["  lead", "", "\ttrail", "  both"]
    );
    assert_eq!(
        string_col(&engine, "e_skip"),
        vec!["  lead  ", "\ttrail\t", "  both  "]
    );

    let err = engine
        .execute("readlines('mixed.txt', 'EmptyLineRule', 'error');")
        .unwrap_err();
    assert!(err.contains("empty line"), "{err}");

    // Escaped CR LF also recognizes lone CR/LF, while literal delimiters stay literal.
    fs::write(dir.path().join("crlf.txt"), b"first\r\nsecond\r\n").unwrap();
    fs::write(dir.path().join("separators.txt"), b"first|second|").unwrap();
    engine
        .execute(
            r#"
        crlf = readlines('crlf.txt', 'LineEnding', "\r\n");
        mixed = readlines('mixed.txt', 'LineEnding', '\r\n');
        pipes = readlines('separators.txt', 'LineEnding', '|');
        writelines(["first"; "second"], 'lf.txt', 'LineEnding', '\n');
        lf = readlines('lf.txt', 'LineEnding', '\n');
    "#,
        )
        .unwrap();
    assert_eq!(string_col(&engine, "crlf"), vec!["first", "second"]);
    assert_eq!(string_col(&engine, "mixed"), string_col(&engine, "w_pres"));
    assert_eq!(string_col(&engine, "pipes"), vec!["first", "second"]);
    assert_eq!(string_col(&engine, "lf"), vec!["first", "second"]);
    assert_eq!(
        fs::read(dir.path().join("lf.txt")).unwrap(),
        b"first\nsecond\n"
    );

    // Custom LineEnding and ISO-8859-1 Encoding.
    engine
        .execute(
            r#"
            writelines(["café"; "naïve"], 'latin.txt', 'Encoding', 'ISO-8859-1', 'LineEnding', "\r\n");
            lat = readlines('latin.txt', 'Encoding', 'ISO-8859-1', 'LineEnding', "\r\n");
            "#,
        )
        .unwrap();
    assert_eq!(string_col(&engine, "lat"), vec!["café", "naïve"]);
    let raw = fs::read(dir.path().join("latin.txt")).unwrap();
    assert_eq!(raw, b"caf\xe9\r\nna\xefve\r\n");
}

#[test]
fn exist_checks_builtins_path_files_vars_folders_and_sweeps_functions() {
    let dir = tempfile::tempdir().unwrap();
    let path_dir = tempfile::tempdir().unwrap();
    fs::write(
        path_dir.path().join("my_user_fn.m"),
        "function y = my_user_fn(x); y = x; end\n",
    )
    .unwrap();
    fs::create_dir(dir.path().join("sub_folder")).unwrap();

    let mut engine = Engine::default();
    engine.set_current_folder(dir.path()).unwrap();
    engine.set_function_directory(dir.path()).unwrap();
    engine
        .execute(&format!(
            "addpath('{}'); my_var = 42; e_var = exist('my_var'); e_dir = exist('sub_folder'); e_file = exist('my_user_fn'); e_wl = exist('writelines'); e_wl_b = exist('writelines', 'builtin');",
            path_dir.path().display()
        ))
        .unwrap();

    assert_eq!(scalar(&engine, "e_var"), 1.);
    assert_eq!(scalar(&engine, "e_dir"), 7.);
    assert_eq!(scalar(&engine, "e_file"), 2.);
    assert_eq!(scalar(&engine, "e_wl"), 5.);
    assert_eq!(scalar(&engine, "e_wl_b"), 5.);

    // Sweep ALL of session::FUNCTIONS in a clean folder with no shadowing files:
    // program-file functions report 2 (0 for 'builtin'), all others report 5 (5 for 'builtin').
    let program_files: &[&str] = &[
        "rms",
        "isoutlier",
        "filloutliers",
        "rmoutliers",
        "detectImportOptions",
        "setvartype",
        "getvaropts",
        "setvaropts",
        "preview",
        "readtimetable",
    ];
    let clean_dir = tempfile::tempdir().unwrap();
    let mut mismatches = Vec::new();
    for &name in matrust::session::FUNCTIONS {
        let mut sweep_engine = Engine::default();
        sweep_engine.set_current_folder(clean_dir.path()).unwrap();
        sweep_engine
            .set_function_directory(clean_dir.path())
            .unwrap();
        let quoted = name.replace('\'', "''");
        let code = format!("r = exist('{quoted}'); rb = exist('{quoted}', 'builtin');");
        sweep_engine.execute(&code).unwrap();
        let (expected, expected_builtin) = if program_files.contains(&name) {
            (2., 0.)
        } else {
            (5., 5.)
        };
        let got = scalar(&sweep_engine, "r");
        let got_builtin = scalar(&sweep_engine, "rb");
        if got != expected || got_builtin != expected_builtin {
            mismatches.push((name, got, expected, got_builtin, expected_builtin));
        }
    }
    assert!(
        mismatches.is_empty(),
        "exist returned unexpected codes for FUNCTIONS entries: {mismatches:?}"
    );
}

#[test]
fn ferror_reports_and_clears_or_rejects_invalid_fid() {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = Engine::default();
    engine.set_current_folder(dir.path()).unwrap();
    engine.set_function_directory(dir.path()).unwrap();

    engine
        .execute(
            r#"
            fid = fopen('sample.txt', 'w');
            m1 = ferror(fid);
            [m2, n2] = ferror(fid);
            m3 = ferror(fid, 'clear');
            fclose(fid);
            "#,
        )
        .unwrap();
    assert_eq!(char_text(&engine, "m1"), "");
    assert_eq!(char_text(&engine, "m2"), "");
    assert_eq!(scalar(&engine, "n2"), 0.);
    assert_eq!(char_text(&engine, "m3"), "");

    fs::write(dir.path().join("sample.txt"), b"12\n34\n").unwrap();
    engine
        .execute(
            r#"
        fid = fopen('sample.txt', 'r+');
        seek_status = fseek(fid, -5, 'bof');
        [seek_msg, seek_num] = ferror(fid);
        [old_msg, old_num] = ferror(fid, 'clear');
        [clear_msg, clear_num] = ferror(fid);
    "#,
        )
        .unwrap();
    assert_eq!(scalar(&engine, "seek_status"), -1.);
    assert_eq!(
        char_text(&engine, "seek_msg"),
        "fseek: failed to seek to requested position"
    );
    assert_eq!(scalar(&engine, "seek_num"), -1.);
    assert_eq!(
        char_text(&engine, "old_msg"),
        char_text(&engine, "seek_msg")
    );
    assert_eq!(scalar(&engine, "old_num"), scalar(&engine, "seek_num"));
    assert_eq!(char_text(&engine, "clear_msg"), "");
    assert_eq!(scalar(&engine, "clear_num"), 0.);

    for read in [
        "fgetl(fid)",
        "fgets(fid)",
        "fread(fid)",
        "fscanf(fid, '%f')",
        "textscan(fid, '%f')",
    ] {
        engine
            .execute(&format!(
                r#"
            fseek(fid, 0, 'eof'); {read};
            [eof_msg, eof_num] = ferror(fid);
            [old_msg, old_num] = ferror(fid, 'clear');
            [clear_msg, clear_num] = ferror(fid);
            {read};
            fseek(fid, 0, 'bof');
            [success_msg, success_num] = ferror(fid);
        "#
            ))
            .unwrap();
        assert!(!char_text(&engine, "eof_msg").is_empty(), "{read}");
        assert_ne!(scalar(&engine, "eof_num"), 0., "{read}");
        assert_eq!(char_text(&engine, "old_msg"), char_text(&engine, "eof_msg"));
        assert_eq!(scalar(&engine, "old_num"), scalar(&engine, "eof_num"));
        for (msg, num) in [("clear_msg", "clear_num"), ("success_msg", "success_num")] {
            assert_eq!(char_text(&engine, msg), "", "{read}");
            assert_eq!(scalar(&engine, num), 0., "{read}");
        }
    }
    // Successful reads, writes and rewind each reset an earlier error on this fid.
    for io in [
        "fgetl(fid)",
        "fgets(fid)",
        "fread(fid, 1)",
        "fscanf(fid, '%f', 1)",
        "textscan(fid, '%f', 1)",
        "fwrite(fid, 49)",
        "fprintf(fid, '1')",
        "frewind(fid)",
        "ftell(fid)",
    ] {
        engine
            .execute(&format!(
                "fseek(fid, 0, 'bof'); fseek(fid, -5, 'bof'); {io}; [m, n] = ferror(fid);"
            ))
            .unwrap();
        assert_eq!(char_text(&engine, "m"), "", "{io}");
        assert_eq!(scalar(&engine, "n"), 0., "{io}");
    }
    // Returning the last terminated line succeeds; the next read encounters EOF.
    engine
        .execute("fseek(fid, -3, 'eof'); fseek(fid, -99, 'bof'); fgetl(fid); [m, n] = ferror(fid);")
        .unwrap();
    assert_eq!(char_text(&engine, "m"), "");
    assert_eq!(scalar(&engine, "n"), 0.);
    engine.execute("fclose(fid);").unwrap();

    for bad in ["ferror(-1)", "ferror(999)", "ferror(fid)"] {
        let err = engine.execute(bad).unwrap_err();
        assert!(err.contains("Invalid file identifier"), "{bad}: {err}");
    }
}

#[test]
fn dos_and_unix_match_system_path_and_echo() {
    let mut engine = Engine::default();
    let variable_command = if cfg!(windows) {
        "echo %MATRUST_7B_TEST%"
    } else {
        "echo $MATRUST_7B_TEST"
    };
    let res = engine
        .execute(&format!(
            "setenv('MATRUST_7B_TEST', 'hello7b'); \
             [s1, o1] = dos('{variable_command}'); \
             [s2, o2] = unix('{variable_command}', '-echo');"
        ))
        .unwrap();
    assert_eq!(scalar(&engine, "s1"), 0.);
    assert_eq!(scalar(&engine, "s2"), 0.);
    assert_eq!(char_text(&engine, "o1").trim(), "hello7b");
    assert_eq!(char_text(&engine, "o2").trim(), "hello7b");
    assert!(
        res.output.iter().any(|l| l.contains("hello7b")),
        "expected -echo output in {:?}",
        res.output
    );
}

#[test]
fn cli_edit_errors_and_web_prints_or_rejects_bad_schemes() {
    let mut engine = Engine::default();
    for cmd in ["edit", "edit foo", "edit('foo.m')"] {
        let err = engine.execute(cmd).unwrap_err();
        assert!(
            err.contains("edit needs the MatRust desktop"),
            "{cmd}: {err}"
        );
    }

    let res = engine
        .execute("st = web('https://example.com', '-browser');")
        .unwrap();
    assert_eq!(scalar(&engine, "st"), 0.);
    assert_eq!(res.output, vec!["Open in a browser: https://example.com"]);

    let statement = engine.execute("web('https://example.com')").unwrap();
    assert_eq!(
        statement.output,
        vec!["Open in a browser: https://example.com"]
    );
    assert!(engine.array("ans").is_none());

    for bad in [
        "web('file:///etc/passwd')",
        "web('javascript:alert(1)')",
        "web('ftp://example.com')",
        "web('custom:handler')",
    ] {
        let err = engine.execute(bad).unwrap_err();
        assert!(err.contains("http:// and https://"), "{bad}: {err}");
    }

    for url in [
        "https://example.com/a b",
        "https://example.com/\"bad",
        "https://example.com/é",
        "https://example.com/<bad>",
        "https://example.com/\\bad",
    ] {
        let err = engine.execute(&format!("web('{url}')")).unwrap_err();
        assert!(err.contains("RFC 3986"), "{url}: {err}");
    }

    // Also verify -batch CLI process behavior for edit and web.
    let bin = env!("CARGO_BIN_EXE_matrust");
    let out = Command::new(bin)
        .args(["-batch", "edit foo.m"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("edit needs the MatRust desktop"),
        "{stderr}"
    );
}

#[test]
fn ferror_only_sets_eof_when_read_asks_for_more_than_available() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("one.txt"), b"ab").unwrap();

    let mut engine = Engine::default();
    engine.set_current_folder(dir.path()).unwrap();
    engine.set_function_directory(dir.path()).unwrap();

    engine
        .execute(
            r#"
        fid = fopen('one.txt', 'r');
        fseek(fid, -1, 'eof');
        [v1, c1] = fscanf(fid, '%c', 1);
        [m1, n1] = ferror(fid);
        fseek(fid, -1, 'eof');
        [v2, c2] = fscanf(fid, '%c', 2);
        [m2, n2] = ferror(fid);
        fseek(fid, -1, 'eof');
        t1 = textscan(fid, '%s', 1);
        [mt1, nt1] = ferror(fid);
        fseek(fid, -1, 'eof');
        t2 = textscan(fid, '%s', 2);
        [mt2, nt2] = ferror(fid);
        fclose(fid);
    "#,
        )
        .unwrap();

    assert_eq!(char_text(&engine, "v1"), "b");
    assert_eq!(scalar(&engine, "c1"), 1.);
    assert_eq!(char_text(&engine, "m1"), "");
    assert_eq!(scalar(&engine, "n1"), 0.);

    assert_eq!(char_text(&engine, "v2"), "b");
    assert_eq!(scalar(&engine, "c2"), 1.);
    assert!(!char_text(&engine, "m2").is_empty());
    assert_ne!(scalar(&engine, "n2"), 0.);

    assert_eq!(char_text(&engine, "mt1"), "");
    assert_eq!(scalar(&engine, "nt1"), 0.);
    assert!(!char_text(&engine, "mt2").is_empty());
    assert_ne!(scalar(&engine, "nt2"), 0.);
}

#[cfg(unix)]
#[test]
fn readlines_rejects_fifo_without_blocking() {
    use std::io::ErrorKind;
    use std::process::Command;
    use std::sync::mpsc;
    use std::time::Duration;

    let dir = tempfile::tempdir().unwrap();
    let fifo = dir.path().join("pipe.txt");
    match Command::new("mkfifo").arg(&fifo).status() {
        Ok(status) => assert!(status.success(), "mkfifo failed: {status:?}"),
        Err(e) if e.kind() == ErrorKind::NotFound => return,
        Err(e) => panic!("mkfifo failed: {e}"),
    }

    let path = dir.path().to_path_buf();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut engine = Engine::default();
        engine.set_current_folder(&path).unwrap();
        engine.set_function_directory(&path).unwrap();
        let res = engine.execute("readlines('pipe.txt');");
        let _ = tx.send(res.err().unwrap_or_default());
    });

    let err = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("readlines hung on FIFO");
    assert!(
        err.contains("Not a regular file"),
        "unexpected error: {err}"
    );
}
