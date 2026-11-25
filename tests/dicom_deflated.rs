use matrust::engine::Engine;
use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/dicom")
        .join(name)
}

#[test]
fn deflated_public_baseline() {
    let mut engine = Engine::default();
    engine
        .execute(&format!(
            "X=dicomread('{}');",
            fixture("deflated_baseline.dcm").display()
        ))
        .unwrap();
    let a = engine.array("X").unwrap();
    assert_eq!(a.class(), "uint16");
    assert_eq!(a.dims().as_ref(), [2, 3]);
    assert_eq!(a.scalar(0, 2).unwrap(), matrust::Scalar::Integer(65535));
}

fn manifest() -> serde_json::Value {
    serde_json::from_slice(include_bytes!("fixtures/dicom/deflated_manifest.json")).unwrap()
}
fn text(engine: &Engine, name: &str) -> String {
    match engine.array(name).unwrap() {
        matrust::Array::Character(c) => c
            .to_utf8(&std::sync::atomic::AtomicBool::new(false))
            .unwrap(),
        other => panic!("expected text, got {}", other.class()),
    }
}
fn number(engine: &Engine, name: &str) -> f64 {
    engine
        .array(name)
        .unwrap()
        .scalar(0, 0)
        .unwrap()
        .into_real()
        .unwrap()
}
fn call(engine: &mut Engine, function: &str, name: &str, output: &str) -> Result<(), String> {
    let path = fixture(name).to_string_lossy().replace('\'', "''");
    engine
        .execute(&format!("{output}={function}('{path}');"))
        .map(|_| ())
}

#[test]
fn independent_metadata_pixels_shapes_and_backings() {
    for case in manifest()["valid"].as_array().unwrap() {
        let mut engine = Engine::default();
        for (kind, info_name) in [("native", "n"), ("deflated", "i")] {
            let name = case[kind].as_str().unwrap();
            let expected: serde_json::Value =
                serde_json::from_slice(&std::fs::read(fixture(&format!("{name}.json"))).unwrap())
                    .unwrap();
            call(&mut engine, "dicominfo", &format!("{name}.dcm"), info_name).unwrap();
            for (key, want) in case["fields"].as_object().unwrap() {
                engine.execute(&format!("q={info_name}.{key};")).unwrap();
                if let Some(s) = want.as_str() {
                    assert_eq!(text(&engine, "q"), s, "{name}: {key}");
                } else {
                    let values = want
                        .as_array()
                        .cloned()
                        .unwrap_or_else(|| vec![want.clone()]);
                    let got = engine.array("q").unwrap();
                    assert_eq!(got.len(), values.len(), "{name}: {key}");
                    for (k, v) in values.iter().enumerate() {
                        assert_eq!(
                            got.scalar(k, 0).unwrap().into_real().unwrap(),
                            v.as_f64().unwrap(),
                            "{name}: {key}[{k}]"
                        );
                    }
                }
            }
            engine.execute(&format!("bytes={info_name}.FileSize; filename={info_name}.Filename; uid={info_name}.TransferSyntaxUID; c=class({info_name}.SelectorSSValue);")).unwrap();
            assert_eq!(
                number(&engine, "bytes"),
                std::fs::metadata(fixture(&format!("{name}.dcm")))
                    .unwrap()
                    .len() as f64
            );
            assert_eq!(
                text(&engine, "filename"),
                fixture(&format!("{name}.dcm")).to_string_lossy()
            );
            assert_eq!(
                text(&engine, "uid"),
                if kind == "native" {
                    "1.2.840.10008.1.2.1"
                } else {
                    "1.2.840.10008.1.2.1.99"
                }
            );
            assert_eq!(text(&engine, "c"), "int16");
            for source in [
                format!(
                    "X=dicomread('{path}');",
                    path = fixture(&format!("{name}.dcm"))
                        .to_string_lossy()
                        .replace('\'', "''")
                ),
                format!(
                    "{info_name}.Rows=999; {info_name}.Columns=999; {info_name}.FileSize=1; {info_name}.TransferSyntaxUID='fake'; {info_name}.PixelData=123; X=dicomread({info_name});"
                ),
            ] {
                engine.execute(&source).unwrap();
                engine.execute("flat=X(:); c=class(X);").unwrap();
                assert_eq!(
                    text(&engine, "c"),
                    expected["class"].as_str().unwrap(),
                    "{name}"
                );
                let dims: Vec<usize> = expected["size"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_u64().unwrap() as usize)
                    .collect();
                assert_eq!(engine.array("X").unwrap().dims().as_ref(), dims, "{name}");
                for (k, v) in expected["column_major"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .enumerate()
                {
                    assert_eq!(
                        engine.array("flat").unwrap().scalar(k, 0).unwrap(),
                        matrust::Scalar::Integer(v.as_i64().unwrap() as i128),
                        "{name}[{k}]"
                    );
                }
                let mut axes = dims.clone();
                axes.resize(4, 1);
                engine
                    .execute(&format!(
                        "last=X({},{},{},{});",
                        axes[0], axes[1], axes[2], axes[3]
                    ))
                    .unwrap();
                assert_eq!(
                    number(&engine, "last"),
                    expected["column_major"]
                        .as_array()
                        .unwrap()
                        .last()
                        .unwrap()
                        .as_f64()
                        .unwrap(),
                    "{name} four-axis indexing"
                );
                engine
                    .execute("indexed=X(2,3); index_flat=flat(6);")
                    .unwrap();
                assert_eq!(number(&engine, "indexed"), number(&engine, "index_flat"));
            }
        }
    }
}

#[test]
fn malformed_streams_and_dataset_reject_without_partial_success() {
    for (name, expected) in manifest()["invalid"].as_object().unwrap() {
        for function in ["dicominfo", "dicomread"] {
            let mut engine = Engine::default();
            let error = call(&mut engine, function, &format!("{name}.dcm"), "X").unwrap_err();
            assert!(
                error.contains(expected.as_str().unwrap()),
                "{function} {name}: {error}"
            );
            assert!(engine.array("X").is_none(), "{name}");
        }
    }
    for name in manifest()["padding"].as_array().unwrap() {
        let name = format!("{}.dcm", name.as_str().unwrap());
        call(&mut Engine::default(), "dicomread", &name, "X").unwrap();
    }
}

#[test]
fn inflated_offsets_memory_pixel_admission_and_work_limit() {
    let path = fixture("deflated_large.dcm");
    assert!(std::fs::metadata(&path).unwrap().len() < 8192);
    let mut engine = Engine::default();
    call(&mut engine, "dicominfo", "deflated_large.dcm", "i").unwrap();
    engine
        .execute(
            "after=i.DigitalSignaturesSequence.Item_1.PatientID; X=dicomread(i); last=X(512,1025);",
        )
        .unwrap();
    assert_eq!(text(&engine, "after"), "AFTER-PIXEL-É");
    assert_eq!(number(&engine, "last"), 4660.);
    assert_eq!(engine.array("X").unwrap().dims().as_ref(), [512, 1025]);
    // Inflation fits, but retained backing + pixel scratch + typed output/copy do not.
    let limits = matrust::engine::Limits {
        memory_bytes: 7 * 1024 * 1024 / 2,
        ..Default::default()
    };
    let mut limited = Engine::with_limits(limits);
    call(&mut limited, "dicominfo", "deflated_large.dcm", "i").unwrap();
    let error = call(&mut limited, "dicomread", "deflated_large.dcm", "X").unwrap_err();
    assert!(
        error.contains("decoded image") && error.contains("budget"),
        "{error}"
    );
    assert!(limited.array("X").is_none());
    for function in ["dicominfo", "dicomread"] {
        for limits in [
            matrust::engine::Limits {
                memory_bytes: 64 * 1024,
                ..Default::default()
            },
            matrust::engine::Limits {
                evaluation_units: Some(100),
                ..Default::default()
            },
        ] {
            let mut limited = Engine::with_limits(limits);
            let error = call(&mut limited, function, "deflated_large.dcm", "X").unwrap_err();
            assert!(
                error.contains("memory") || error.contains("budget"),
                "{function}: {error}"
            );
            assert!(limited.array("X").is_none());
        }
    }
}

#[test]
fn absolute_decoded_cap_with_large_engine_budget() {
    let mut engine = Engine::with_limits(matrust::engine::Limits {
        memory_bytes: 1024 * 1024 * 1024,
        evaluation_units: None,
        ..Default::default()
    });
    let error = call(&mut engine, "dicominfo", "deflated_bad_bomb.dcm", "X").unwrap_err();
    assert!(error.contains("256 MiB cap"), "{error}");
    assert!(engine.array("X").is_none());
}

#[test]
fn info_rereads_current_file_and_public_dispatch() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("synthetic.dcm");
    let original = include_bytes!("fixtures/dicom/deflated_baseline.dcm");
    std::fs::write(&path, original).unwrap();
    let mut engine = Engine::default();
    engine.set_file_directory(dir.path()).unwrap();
    engine.execute("i=dicominfo('synthetic.dcm'); f=@dicomread; X=f(i); dicomread(\"synthetic.dcm\"); X=ans;").unwrap();
    assert_eq!(engine.array("X").unwrap().class(), "uint16");
    assert_eq!(std::fs::read(&path).unwrap(), original);
    std::fs::write(
        &path,
        include_bytes!("fixtures/dicom/deflated_bad_corrupt.dcm"),
    )
    .unwrap();
    assert!(
        engine
            .execute("Y=dicomread(i);")
            .unwrap_err()
            .contains("corrupt")
    );
    assert!(engine.array("Y").is_none());
    std::fs::write(&path, original).unwrap();
    let source = format!(
        "X=dicomread(dicominfo('{}')); class(X), double(X(1,3))",
        path.to_string_lossy().replace('\'', "''")
    );
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_matrust"))
        .args(["--eval", &source])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let output = String::from_utf8_lossy(&result.stdout);
    assert!(
        output.contains("uint16") && output.contains("65535"),
        "{output}"
    );
}

#[test]
#[ignore = "manual optimized deflate timing with 1 and N caller workers"]
fn deflated_release_timing() {
    let workers: usize = std::env::var("DICOM_BENCH_WORKERS")
        .unwrap_or_else(|_| "1".into())
        .parse()
        .unwrap();
    rayon::ThreadPoolBuilder::new().num_threads(workers).build().unwrap().install(|| {
        for function in ["dicominfo","dicomread"] {
            let mut engine=Engine::default();
            call(&mut engine,function,"deflated_large.dcm","X").unwrap();
            let start=std::time::Instant::now();
            for _ in 0..100 { call(&mut engine,function,"deflated_large.dcm","X").unwrap(); }
            eprintln!("DICOM deflated: function={function}, workers={workers}, iterations=100, decoded_pixel_bytes=1049600 elapsed_ms={:.3}",start.elapsed().as_secs_f64()*1000.);
        }
    });
}
