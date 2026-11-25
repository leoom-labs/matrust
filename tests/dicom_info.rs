//! Synthetic Part 10 metadata regressions, independent pydicom expectations.
use matrust::Engine;

#[test]
fn explicit_le_public_baseline() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("synthetic.dcm");
    std::fs::write(&path, include_bytes!("fixtures/dicom/baseline.dcm")).unwrap();
    let mut engine = Engine::default();
    engine
        .execute(&format!(
            "i=dicominfo('{}'); w=i.Width; h=i.Height; b=i.BitDepth;",
            path.to_string_lossy().replace('\'', "''")
        ))
        .unwrap();
    for (name, expected) in [("w", 3.), ("h", 2.), ("b", 16.)] {
        assert_eq!(
            engine
                .array(name)
                .unwrap()
                .scalar(0, 0)
                .unwrap()
                .into_real()
                .unwrap(),
            expected
        );
    }
}

fn fixture(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/dicom")
        .join(name)
}

fn call(engine: &mut Engine, path: &std::path::Path) {
    engine
        .execute(&format!(
            "i=dicominfo('{}');",
            path.to_string_lossy().replace('\'', "''")
        ))
        .unwrap();
}

fn text(engine: &Engine, name: &str) -> String {
    match engine.array(name).unwrap() {
        matrust::Array::Character(c) => c
            .to_utf8(&std::sync::atomic::AtomicBool::new(false))
            .unwrap(),
        other => panic!("expected text, got {}", other.class()),
    }
}

#[test]
fn three_syntaxes_match_independent_metadata() {
    for syntax in ["explicit_le", "implicit_le", "explicit_be"] {
        let expected: serde_json::Value =
            serde_json::from_slice(&std::fs::read(fixture(&format!("{syntax}.json"))).unwrap())
                .unwrap();
        let mut engine = Engine::default();
        call(&mut engine, &fixture(&format!("{syntax}.dcm")));
        let fields = expected["fields"].as_object().unwrap();
        assert!(fields.len() >= 20);
        for (key, value) in fields {
            engine.execute(&format!("q=i.{key};")).unwrap();
            if let Some(want) = value.as_str() {
                assert_eq!(text(&engine, "q"), want, "{syntax} {key}");
            } else {
                let want = value
                    .as_array()
                    .cloned()
                    .unwrap_or_else(|| vec![value.clone()]);
                let got = engine.array("q").unwrap();
                assert_eq!(got.len(), want.len(), "{syntax} {key}");
                for (index, want) in want.into_iter().enumerate() {
                    let scalar = got.scalar(index % got.rows(), index / got.rows()).unwrap();
                    if let matrust::Scalar::Integer(actual) = scalar {
                        let expected = want
                            .as_i64()
                            .map(i128::from)
                            .or_else(|| want.as_u64().map(i128::from))
                            .unwrap();
                        assert_eq!(actual, expected, "{syntax} {key}[{index}]");
                    } else {
                        assert_eq!(
                            scalar.into_real().unwrap(),
                            want.as_f64().unwrap(),
                            "{syntax} {key}[{index}]"
                        );
                    }
                }
            }
        }
        for (key, want) in expected["classes"].as_object().unwrap() {
            engine.execute(&format!("q=class(i.{key});")).unwrap();
            assert_eq!(text(&engine, "q"), want.as_str().unwrap(), "{syntax} {key}");
        }
        engine.execute("p=isfield(i,'PixelData'); u=isfield(i,'Unknown_0011_1010'); after=isfield(i,'DigitalSignaturesSequence');").unwrap();
        for (key, want) in [("p", 0.), ("u", 0.), ("after", 1.)] {
            assert_eq!(
                engine
                    .array(key)
                    .unwrap()
                    .scalar(0, 0)
                    .unwrap()
                    .into_real()
                    .unwrap(),
                want
            );
        }
    }
}

#[test]
fn nested_defined_and_undefined_sequences_are_structural() {
    for syntax in ["explicit_le", "implicit_le", "explicit_be"] {
        for length in ["defined", "undefined", "mixed_defined", "mixed_undefined"] {
            let mut engine = Engine::default();
            call(&mut engine, &fixture(&format!("{syntax}_{length}.dcm")));
            engine.execute("a=i.ReferencedStudySequence.Item_1.PatientID; b=i.ReferencedStudySequence.Item_1.ReferencedImageSequence.Item_1.PatientID; utf=i.ReferencedStudySequence.Item_1.ReferencedImageSequence.Item_1.PatientName.FamilyName; c=fieldnames(i.ReferencedStudySequence); n=numel(c); empty=fieldnames(i.ReferencedStudySequence.Item_2); z=numel(empty);").unwrap();
            assert_eq!(text(&engine, "a"), "OUTER-SYNTHETIC");
            assert_eq!(text(&engine, "b"), "INNER-SYNTHETIC");
            assert_eq!(text(&engine, "utf"), "Synthétique");
            for (key, want) in [("n", 2.), ("z", 0.)] {
                assert_eq!(
                    engine
                        .array(key)
                        .unwrap()
                        .scalar(0, 0)
                        .unwrap()
                        .into_real()
                        .unwrap(),
                    want
                );
            }
        }
    }
}

#[test]
fn malformed_inputs_fail_without_panicking() {
    let cases: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/dicom/malformed.json")).unwrap();
    assert!(cases.as_object().unwrap().len() >= 8);
    for (name, want) in cases.as_object().unwrap() {
        let path = fixture(&format!("bad_{name}.dcm"));
        let result = std::panic::catch_unwind(|| {
            Engine::default().execute(&format!(
                "i=dicominfo('{}');",
                path.to_string_lossy().replace('\'', "''")
            ))
        });
        let error = result.expect("parser must not panic").expect_err(name);
        assert!(error.contains(want.as_str().unwrap()), "{name}: {error}");
    }
}

#[test]
fn arguments_and_options_are_checked() {
    for (source, want) in [
        ("dicominfo()", "input"),
        ("dicominfo(42)", "filename"),
        ("dicominfo('missing','dictionary','custom')", "options"),
        ("dicominfo('missing','UseVRHeuristic',true)", "options"),
        ("[a,b]=dicominfo('missing')", "output"),
        ("dicominfo('https://example.invalid/file')", "Remote"),
    ] {
        let error = Engine::default().execute(source).expect_err(source);
        assert!(error.contains(want), "{source}: {error}");
    }
    assert!(matrust::session::FUNCTIONS.contains(&"dicominfo"));
    let mut engine = Engine::default();
    engine
        .execute("q=exist('dicominfo','builtin'); f=@dicominfo;")
        .unwrap();
    assert_eq!(
        engine
            .array("q")
            .unwrap()
            .scalar(0, 0)
            .unwrap()
            .into_real()
            .unwrap(),
        5.
    );
}

#[test]
fn file_and_workspace_admission_are_bounded() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("oversized.dcm");
    let file = std::fs::File::create(&path).unwrap();
    file.set_len(256 * 1024 * 1024 + 1).unwrap();
    let error = Engine::default()
        .execute(&format!(
            "i=dicominfo('{}');",
            path.to_string_lossy().replace('\'', "''")
        ))
        .unwrap_err();
    assert!(error.contains("byte limit"), "{error}");
    let mut engine = Engine::with_limits(matrust::engine::Limits {
        memory_bytes: 128,
        ..matrust::engine::Limits::default()
    });
    let path = fixture("explicit_le.dcm");
    let error = engine
        .execute(&format!(
            "i=dicominfo('{}');",
            path.to_string_lossy().replace('\'', "''")
        ))
        .unwrap_err();
    assert!(
        error.contains("memory") || error.contains("budget"),
        "{error}"
    );
}

#[test]
fn large_native_pixel_payload_is_skipped_and_file_is_unchanged() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sparse-native.dcm");
    let mut prefix = include_bytes!("fixtures/dicom/baseline.dcm").to_vec();
    prefix.truncate(prefix.len() - 12);
    let offset = prefix.len();
    let length = 64u32 * 1024 * 1024;
    prefix[offset - 4..].copy_from_slice(&length.to_le_bytes());
    let mut file = std::fs::File::create(&path).unwrap();
    file.write_all(&prefix).unwrap();
    file.set_len(offset as u64 + u64::from(length)).unwrap();
    drop(file);
    let mut engine = Engine::default();
    call(&mut engine, &path);
    engine
        .execute("s=i.FileSize; p=isfield(i,'PixelData'); absent=isfield(i,'PatientName');")
        .unwrap();
    assert_eq!(
        engine
            .array("s")
            .unwrap()
            .scalar(0, 0)
            .unwrap()
            .into_real()
            .unwrap(),
        offset as f64 + f64::from(length)
    );
    for name in ["p", "absent"] {
        assert_eq!(
            engine
                .array(name)
                .unwrap()
                .scalar(0, 0)
                .unwrap()
                .into_real()
                .unwrap(),
            0.
        );
    }
    use std::io::Read;
    let mut actual = vec![0; offset];
    std::fs::File::open(&path)
        .unwrap()
        .read_exact(&mut actual)
        .unwrap();
    assert_eq!(actual, prefix);
    assert_eq!(
        std::fs::metadata(&path).unwrap().len(),
        offset as u64 + u64::from(length)
    );
}

/// Serial parser timing under a caller-owned pool; no pool is created by DICOM.
#[test]
#[ignore = "manual optimized metadata timing, 1 and N caller workers"]
fn metadata_release_timing() {
    let workers: usize = std::env::var("DICOM_BENCH_WORKERS")
        .unwrap_or_else(|_| "1".into())
        .parse()
        .unwrap();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .unwrap();
    let path = fixture("explicit_le_undefined.dcm");
    pool.install(|| {
        let mut engine = Engine::default();
        let source = format!(
            "i=dicominfo('{}');",
            path.to_string_lossy().replace('\'', "''")
        );
        engine.execute(&source).unwrap();
        let start = std::time::Instant::now();
        for _ in 0..1000 {
            engine.execute(&source).unwrap();
        }
        eprintln!(
            "DICOM metadata: workers={workers}, iterations=1000, bytes={}, elapsed_ms={:.3}",
            std::fs::metadata(path).unwrap().len(),
            start.elapsed().as_secs_f64() * 1000.
        );
    });
}

#[test]
fn filename_strings_statement_expression_and_handle_paths() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("synthetic.dcm");
    std::fs::write(&path, include_bytes!("fixtures/dicom/baseline.dcm")).unwrap();
    let mut engine = Engine::default();
    engine.set_file_directory(dir.path()).unwrap();
    for source in [
        "dicominfo('synthetic.dcm'); i=ans;",
        "i=dicominfo(\"synthetic.dcm\");",
        "filename=string('synthetic.dcm'); i=dicominfo(filename);",
        "f=@dicominfo; i=f('synthetic.dcm');",
    ] {
        engine.execute(source).unwrap();
        engine.execute("q=i.Width;").unwrap();
        assert_eq!(
            engine
                .array("q")
                .unwrap()
                .scalar(0, 0)
                .unwrap()
                .into_real()
                .unwrap(),
            3.,
            "{source}"
        );
    }
    for source in [
        "dicominfo({'synthetic.dcm'})",
        "dicominfo([\"a\",\"b\"])",
        "dicominfo(char([97;98]))",
    ] {
        let error = engine.execute(source).unwrap_err();
        assert!(error.contains("filename"), "{error}");
    }
}

#[test]
fn ascii_padding_empty_and_absent_tags_remain_distinct() {
    let mut engine = Engine::default();
    call(&mut engine, &fixture("ascii_missing.dcm"));
    engine.execute("a=i.PatientName.GivenName; b=i.PatientID; c=isempty(i.StudyDescription); d=isempty(i.PixelSpacing); e=isfield(i,'StudyInstanceUID'); f=isfield(i,'Width');").unwrap();
    assert_eq!(text(&engine, "a"), "ASCII");
    assert_eq!(text(&engine, "b"), "  KEEP-LEADING");
    for (name, want) in [("c", 1.), ("d", 1.), ("e", 0.), ("f", 0.)] {
        assert_eq!(
            engine
                .array(name)
                .unwrap()
                .scalar(0, 0)
                .unwrap()
                .into_real()
                .unwrap(),
            want
        );
    }
}

#[test]
fn cli_public_path_reads_synthetic_metadata() {
    let path = fixture("explicit_be.dcm");
    let source = format!(
        "i=dicominfo('{}'); disp([i.Height i.Width i.BitDepth]);",
        path.to_string_lossy().replace('\'', "''")
    );
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_matrust"))
        .args(["--eval", &source])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let values: Vec<f64> = String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .map(|s| s.parse().unwrap())
        .collect();
    assert_eq!(values, [2., 3., 12.]);
}

#[test]
#[cfg(unix)]
fn fifo_inputs_error_without_opening() {
    const CHILD: &str = "MATRUST_DICOM_FIFO_REGRESSION";
    if let Some(path) = std::env::var_os(CHILD) {
        for function in ["dicominfo", "dicomread"] {
            let error = Engine::default()
                .execute(&format!(
                    "x={function}('{}');",
                    std::path::Path::new(&path).display()
                ))
                .unwrap_err();
            assert!(error.contains("regular file"), "{error}");
        }
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let fifo = dir.path().join("pipe.dcm");
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("dicom_info::fifo_inputs_error_without_opening")
        .env(CHILD, &fifo)
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "FIFO child failed: {status}");
            break;
        }
        if std::time::Instant::now() >= deadline {
            let pid = child.id();
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("dicominfo/dicomread blocked opening FIFO (killed own child PID {pid})");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}
