//! Native grayscale/RGB frames checked against synthetic pydicom/NumPy references.
use matrust::Engine;

#[test]
fn explicit_le_public_baseline() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("synthetic.dcm");
    std::fs::write(
        &path,
        include_bytes!("fixtures/dicom/pixels_explicit_le.dcm"),
    )
    .unwrap();
    let mut engine = Engine::default();
    engine
        .execute(&format!(
            "X=dicomread('{}');",
            path.to_string_lossy().replace('\'', "''")
        ))
        .unwrap();
    let a = engine.array("X").unwrap();
    assert_eq!(a.class(), "uint16");
    assert_eq!((a.rows(), a.cols()), (2, 3));
    for (i, expected) in [0, 32768, 1, 42, 65535, 256].into_iter().enumerate() {
        assert_eq!(
            a.scalar(i % 2, i / 2).unwrap(),
            matrust::Scalar::Integer(expected)
        );
    }
}

fn fixture(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/dicom")
        .join(name)
}

fn text(engine: &Engine, name: &str) -> String {
    match engine.array(name).unwrap() {
        matrust::Array::Character(c) => c
            .to_utf8(&std::sync::atomic::AtomicBool::new(false))
            .unwrap(),
        other => panic!("expected text, got {}", other.class()),
    }
}

fn read(
    engine: &mut Engine,
    path: &std::path::Path,
) -> matrust::Result<matrust::engine::Execution> {
    engine.execute(&format!(
        "X=dicomread('{}');",
        path.to_string_lossy().replace('\'', "''")
    ))
}

#[test]
fn independent_pixels_classes_shapes_indexing_and_storage() {
    let manifest: serde_json::Value =
        serde_json::from_slice(include_bytes!("fixtures/dicom/pixels_manifest.json")).unwrap();
    for name in manifest["valid"].as_array().unwrap() {
        let name = name.as_str().unwrap();
        let expected: serde_json::Value =
            serde_json::from_slice(&std::fs::read(fixture(&format!("{name}.json"))).unwrap())
                .unwrap();
        let mut engine = Engine::default();
        read(&mut engine, &fixture(&format!("{name}.dcm"))).unwrap();
        engine.execute("c=class(X); s=size(X); flat=X(:);").unwrap();
        assert_eq!(
            text(&engine, "c"),
            expected["class"].as_str().unwrap(),
            "{name}"
        );
        let rows = expected["size"][0].as_u64().unwrap() as usize;
        let cols = expected["size"][1].as_u64().unwrap() as usize;
        let a = engine.array("X").unwrap();
        assert_eq!((a.rows(), a.cols()), (rows, cols), "{name}");
        for (i, n) in [rows, cols].into_iter().enumerate() {
            assert_eq!(
                engine
                    .array("s")
                    .unwrap()
                    .scalar(0, i)
                    .unwrap()
                    .into_real()
                    .unwrap(),
                n as f64,
                "{name}"
            );
        }
        for (i, value) in expected["column_major"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            let want = matrust::Scalar::Integer(i128::from(value.as_i64().unwrap()));
            assert_eq!(a.scalar(i % rows, i / rows).unwrap(), want, "{name}[{i}]");
            assert_eq!(
                engine.array("flat").unwrap().scalar(i, 0).unwrap(),
                want,
                "{name} linear[{i}]"
            );
        }
        for (i, value) in expected["row_major"].as_array().unwrap().iter().enumerate() {
            engine
                .execute(&format!("v=X({},{});", i / cols + 1, i % cols + 1))
                .unwrap();
            assert_eq!(
                engine.array("v").unwrap().scalar(0, 0).unwrap(),
                matrust::Scalar::Integer(i128::from(value.as_i64().unwrap())),
                "{name} public indexing[{i}]"
            );
        }
    }
}

#[test]
fn filename_info_relative_statement_and_handle_paths() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("synthetic.dcm");
    let original = include_bytes!("fixtures/dicom/pixels_explicit_le.dcm");
    std::fs::write(&path, original).unwrap();
    let mut engine = Engine::default();
    engine.set_file_directory(dir.path()).unwrap();
    for source in [
        "dicomread('synthetic.dcm'); X=ans;",
        "X=dicomread(\"synthetic.dcm\");",
        "filename=string('synthetic.dcm'); X=dicomread(filename);",
        "f=@dicomread; X=f('synthetic.dcm');",
        "i=dicominfo('synthetic.dcm'); X=dicomread(i);",
        "i.Rows=999; i.Columns=999; i.TransferSyntaxUID='fake'; i.PixelData=123; X=dicomread(i);",
        "i.Filename=string('synthetic.dcm'); X=dicomread(i);",
    ] {
        engine.execute(source).unwrap();
        let a = engine.array("X").unwrap();
        assert_eq!(
            (a.rows(), a.cols(), a.class()),
            (2, 3, "uint16"),
            "{source}"
        );
        assert_eq!(a.scalar(0, 2).unwrap(), matrust::Scalar::Integer(65535));
    }
    assert_eq!(std::fs::read(&path).unwrap(), original);
    // Reusing an info struct must validate the current file, not cached metadata.
    std::fs::write(&path, b"not a Part 10 file").unwrap();
    assert!(
        engine
            .execute("X=dicomread(i);")
            .unwrap_err()
            .contains("Part 10")
    );
}

#[test]
fn malformed_native_pixels_are_rejected() {
    let manifest: serde_json::Value =
        serde_json::from_slice(include_bytes!("fixtures/dicom/pixels_manifest.json")).unwrap();
    for (name, expected) in manifest["invalid"].as_object().unwrap() {
        let mut engine = Engine::default();
        let error = read(&mut engine, &fixture(&format!("{name}.dcm"))).unwrap_err();
        assert!(
            error.contains(expected.as_str().unwrap()),
            "{name}: {error}"
        );
        assert!(engine.array("X").is_none(), "{name}: no partial output");
    }
}

#[test]
fn argument_options_outputs_and_info_validation() {
    let mut engine = Engine::default();
    for (source, expected) in [
        ("dicomread()", "input"),
        ("dicomread(42)", "filename"),
        ("dicomread({'a'})", "filename"),
        ("dicomread([\"a\",\"b\"])", "filename"),
        ("dicomread(char([97;98]))", "filename"),
        ("dicomread('a','Frames',1)", "options"),
        ("dicomread('a','UseVRHeuristic',false)", "options"),
        ("[x,y]=dicomread('a')", "output"),
        ("[x,y,z]=dicomread('a')", "output"),
        ("dicomread(struct())", "Filename"),
        ("dicomread(struct('Filename',42))", "Filename"),
        ("dicomread(struct('Filename',{'a','b'}))", "scalar"),
        ("dicomread(struct('Filename',{['a';'b']}))", "Filename"),
        ("dicomread(struct('Filename',string(missing)))", "Filename"),
    ] {
        let error = engine.execute(source).unwrap_err();
        assert!(
            error.to_lowercase().contains(&expected.to_lowercase()),
            "{source}: {error}"
        );
    }
}

#[test]
fn file_and_image_memory_admission() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("oversized.dcm");
    let file = std::fs::File::create(&path).unwrap();
    file.set_len(256 * 1024 * 1024 + 1).unwrap();
    drop(file);
    let error = read(&mut Engine::default(), &path).unwrap_err();
    assert!(error.contains("byte limit"), "{error}");
    let mut engine = Engine::with_limits(matrust::engine::Limits {
        memory_bytes: 128,
        ..matrust::engine::Limits::default()
    });
    let error = read(&mut engine, &fixture("pixels_explicit_le.dcm")).unwrap_err();
    assert!(
        error.contains("memory") || error.contains("budget"),
        "{error}"
    );
}

#[test]
fn real_cli_public_call() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("synthetic.dcm");
    std::fs::write(
        &path,
        include_bytes!("fixtures/dicom/pixels_explicit_le.dcm"),
    )
    .unwrap();
    let source = format!(
        "X=dicomread('{}'); class(X), size(X), double(X(2,3))",
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
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("uint16") && stdout.contains("256"),
        "{stdout}"
    );
    std::fs::write(&path, include_bytes!("fixtures/dicom/frames_baseline.dcm")).unwrap();
    let source = format!(
        "X=dicomread(dicominfo('{}')); class(X), size(X), double(X(2,3,3,2))",
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
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("uint8") && stdout.contains("149"),
        "{stdout}"
    );
}

#[test]
fn pixel_payload_exceeds_metadata_cap_and_obeys_image_budget() {
    let expected: serde_json::Value =
        serde_json::from_slice(include_bytes!("fixtures/dicom/pixels_large.json")).unwrap();
    let mut engine = Engine::default();
    read(&mut engine, &fixture("pixels_large.dcm")).unwrap();
    let a = engine.array("X").unwrap();
    assert_eq!((a.rows(), a.cols()), (512, 1025));
    for sample in expected["samples"].as_array().unwrap() {
        assert_eq!(
            a.scalar(
                sample[0].as_u64().unwrap() as usize,
                sample[1].as_u64().unwrap() as usize
            )
            .unwrap(),
            matrust::Scalar::Integer(i128::from(sample[2].as_i64().unwrap()))
        );
    }
    for limits in [
        matrust::engine::Limits {
            memory_bytes: 1024 * 1024,
            ..matrust::engine::Limits::default()
        },
        matrust::engine::Limits {
            evaluation_units: Some(100),
            ..matrust::engine::Limits::default()
        },
    ] {
        let mut limited = Engine::with_limits(limits);
        let error = read(&mut limited, &fixture("pixels_large.dcm")).unwrap_err();
        assert!(
            error.contains("memory") || error.contains("budget"),
            "{error}"
        );
        assert!(limited.array("X").is_none());
    }
}

#[test]
#[ignore = "manual optimized pixel timing, 1 and N caller workers"]
fn pixels_release_timing() {
    let workers: usize = std::env::var("DICOM_BENCH_WORKERS")
        .unwrap_or_else(|_| "1".into())
        .parse()
        .unwrap();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .unwrap();
    pool.install(|| {
        let mut engine = Engine::default();
        for name in ["pixels_large.dcm", "frames_large_pc0.dcm", "frames_large_pc1.dcm"] {
            let path = fixture(name);
            read(&mut engine, &path).unwrap();
            let start = std::time::Instant::now();
            for _ in 0..100 {
                read(&mut engine, &path).unwrap();
            }
            eprintln!("DICOM pixels: file={name}, workers={workers}, iterations=100, shape={:?}, bytes={}, elapsed_ms={:.3}", engine.array("X").unwrap().dims(), std::fs::metadata(&path).unwrap().len(), start.elapsed().as_secs_f64() * 1000.);
        }
    });
}

#[test]
fn rgb_multiframe_public_baseline() {
    let mut engine = Engine::default();
    read(&mut engine, &fixture("frames_baseline.dcm")).unwrap();
    engine
        .execute("c=class(X); dims=[size(X,1),size(X,2),size(X,3),size(X,4)]; flat=X(:);")
        .unwrap();
    assert_eq!(text(&engine, "c"), "uint8");
    for (i, d) in [2., 3., 3., 2.].into_iter().enumerate() {
        assert_eq!(
            engine
                .array("dims")
                .unwrap()
                .scalar(0, i)
                .unwrap()
                .into_real()
                .unwrap(),
            d
        );
    }
    let expected: serde_json::Value =
        serde_json::from_slice(include_bytes!("fixtures/dicom/frames_baseline.json")).unwrap();
    for (i, v) in expected["column_major"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        assert_eq!(
            engine.array("flat").unwrap().scalar(i, 0).unwrap(),
            matrust::Scalar::Integer(i128::from(v.as_i64().unwrap()))
        );
    }
}

#[test]
fn independent_native_frames_shapes_planes_and_info() {
    let manifest: serde_json::Value =
        serde_json::from_slice(include_bytes!("fixtures/dicom/frames_manifest.json")).unwrap();
    for name in manifest["valid"].as_array().unwrap() {
        let name = name.as_str().unwrap();
        let expected: serde_json::Value =
            serde_json::from_slice(&std::fs::read(fixture(&format!("{name}.json"))).unwrap())
                .unwrap();
        let path = fixture(&format!("{name}.dcm"));
        let quoted = path.to_string_lossy().replace('\'', "''");
        let mut engine = Engine::default();
        for source in [
            format!("X=dicomread('{quoted}');"),
            format!(
                "info=dicominfo('{quoted}'); info.NumberOfFrames=999; info.PlanarConfiguration=99; X=dicomread(info);"
            ),
        ] {
            engine
                .execute(&source)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            engine.execute("c=class(X); dims=[size(X,1),size(X,2),size(X,3),size(X,4)]; count=numel(X); flat=X(:);").unwrap();
            assert_eq!(
                text(&engine, "c"),
                expected["class"].as_str().unwrap(),
                "{name}"
            );
            let dims: Vec<usize> = expected["size"]
                .as_array()
                .unwrap()
                .iter()
                .map(|n| n.as_u64().unwrap() as usize)
                .collect();
            assert_eq!(engine.array("X").unwrap().dims().as_ref(), dims, "{name}");
            for (k, d) in expected["size4"].as_array().unwrap().iter().enumerate() {
                assert_eq!(
                    engine
                        .array("dims")
                        .unwrap()
                        .scalar(0, k)
                        .unwrap()
                        .into_real()
                        .unwrap(),
                    d.as_u64().unwrap() as f64,
                    "{name} size axis {k}"
                );
            }
            let values = expected["column_major"].as_array().unwrap();
            assert_eq!(
                engine
                    .array("count")
                    .unwrap()
                    .scalar(0, 0)
                    .unwrap()
                    .into_real()
                    .unwrap(),
                values.len() as f64,
                "{name}"
            );
            for (i, v) in values.iter().enumerate() {
                assert_eq!(
                    engine.array("flat").unwrap().scalar(i, 0).unwrap(),
                    matrust::Scalar::Integer(i128::from(v.as_i64().unwrap())),
                    "{name} linear {i}"
                );
            }
            let samples = expected["size4"][2].as_u64().unwrap() as usize;
            for (p, values) in expected["planes"].as_array().unwrap().iter().enumerate() {
                engine
                    .execute(&format!(
                        "plane=X(:,:,{},{}); plane_flat=plane(:);",
                        p % samples + 1,
                        p / samples + 1
                    ))
                    .unwrap();
                assert_eq!(
                    engine.array("plane").unwrap().class(),
                    expected["class"].as_str().unwrap(),
                    "{name}"
                );
                assert_eq!(
                    engine.array("plane").unwrap().dims().as_ref(),
                    &dims[..2],
                    "{name}"
                );
                for (i, v) in values.as_array().unwrap().iter().enumerate() {
                    assert_eq!(
                        engine.array("plane_flat").unwrap().scalar(i, 0).unwrap(),
                        matrust::Scalar::Integer(i128::from(v.as_i64().unwrap())),
                        "{name} plane {p}, sample {i}"
                    );
                }
            }
        }
    }
    // Both native encodings agree on independently decoded samples and shape.
    for pair in manifest["pairs"].as_array().unwrap() {
        let mut engine = Engine::default();
        read(
            &mut engine,
            &fixture(&format!("{}.dcm", pair[0].as_str().unwrap())),
        )
        .unwrap();
        engine.execute("first=X;").unwrap();
        read(
            &mut engine,
            &fixture(&format!("{}.dcm", pair[1].as_str().unwrap())),
        )
        .unwrap();
        engine.execute("same=isequal(first,X);").unwrap();
        assert_eq!(
            engine
                .array("same")
                .unwrap()
                .scalar(0, 0)
                .unwrap()
                .into_real()
                .unwrap(),
            1.
        );
    }
}

#[test]
fn malformed_native_frames_are_rejected() {
    let manifest: serde_json::Value =
        serde_json::from_slice(include_bytes!("fixtures/dicom/frames_manifest.json")).unwrap();
    for (name, expected) in manifest["invalid"].as_object().unwrap() {
        let mut engine = Engine::default();
        let error = read(&mut engine, &fixture(&format!("{name}.dcm"))).unwrap_err();
        assert!(
            error.contains(expected.as_str().unwrap()),
            "{name}: {error}"
        );
        assert!(engine.array("X").is_none(), "{name}: no partial output");
    }
    // Current file contents must still be validated when RGB info is reused.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("frames.dcm");
    std::fs::write(&path, include_bytes!("fixtures/dicom/frames_baseline.dcm")).unwrap();
    let mut engine = Engine::default();
    engine.set_file_directory(dir.path()).unwrap();
    engine.execute("info=dicominfo('frames.dcm');").unwrap();
    std::fs::write(
        &path,
        include_bytes!("fixtures/dicom/frames_bad_missing_pc.dcm"),
    )
    .unwrap();
    assert!(
        engine
            .execute("X=dicomread(info);")
            .unwrap_err()
            .contains("PlanarConfiguration")
    );
    assert!(engine.array("X").is_none());
}

#[test]
fn large_native_frames_reference_and_admission() {
    for pc in [0, 1] {
        let name = format!("frames_large_pc{pc}");
        let expected: serde_json::Value =
            serde_json::from_slice(&std::fs::read(fixture(&format!("{name}.json"))).unwrap())
                .unwrap();
        let mut engine = Engine::default();
        read(&mut engine, &fixture(&format!("{name}.dcm"))).unwrap();
        assert_eq!(engine.array("X").unwrap().dims().as_ref(), [128, 257, 3, 2]);
        for sample in expected["samples"].as_array().unwrap() {
            let v: Vec<u64> = sample
                .as_array()
                .unwrap()
                .iter()
                .map(|n| n.as_u64().unwrap())
                .collect();
            engine
                .execute(&format!(
                    "v=X({},{},{},{});",
                    v[0] + 1,
                    v[1] + 1,
                    v[2] + 1,
                    v[3] + 1
                ))
                .unwrap();
            assert_eq!(
                engine.array("v").unwrap().scalar(0, 0).unwrap(),
                matrust::Scalar::Integer(i128::from(v[4]))
            );
        }
        for limits in [
            matrust::engine::Limits {
                memory_bytes: 1024 * 1024,
                ..matrust::engine::Limits::default()
            },
            matrust::engine::Limits {
                evaluation_units: Some(100),
                ..matrust::engine::Limits::default()
            },
        ] {
            let mut engine = Engine::with_limits(limits);
            let error = read(&mut engine, &fixture(&format!("{name}.dcm"))).unwrap_err();
            assert!(
                error.contains("memory") || error.contains("budget"),
                "{error}"
            );
            assert!(engine.array("X").is_none());
        }
    }
}
