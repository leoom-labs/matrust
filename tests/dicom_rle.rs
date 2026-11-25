//! RLE Lossless (1.2.840.10008.1.2.5) reads checked against synthetic pydicom/NumPy references.
//! Fixtures and expected values: tool/refs/dicom/rle.py (never Rust output).
use matrust::engine::Engine;
use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/dicom")
        .join(name)
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(include_bytes!("fixtures/dicom/rle_manifest.json")).unwrap()
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
fn rle_public_baseline() {
    let mut engine = Engine::default();
    engine
        .execute(&format!(
            "X=dicomread('{}');",
            fixture("rle_baseline.dcm").display()
        ))
        .unwrap();
    let a = engine.array("X").unwrap();
    assert_eq!(a.class(), "uint16");
    assert_eq!(a.dims().as_ref(), [2, 3]);
    for (i, expected) in [1, 7, 257, 4096, 65535, 12345].into_iter().enumerate() {
        assert_eq!(
            a.scalar(i % 2, i / 2).unwrap(),
            matrust::Scalar::Integer(expected)
        );
    }
}

#[test]
fn fixtures_match_generated_hashes() {
    use sha2::Digest;
    let hashes = manifest()["sha256"].as_object().unwrap().clone();
    assert!(hashes.len() >= 50);
    for (name, want) in hashes {
        let got = sha2::Sha256::digest(std::fs::read(fixture(&name)).unwrap());
        let got: String = got.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(got, want.as_str().unwrap(), "{name}");
    }
}

fn check_pixels(engine: &mut Engine, case: &serde_json::Value) {
    let name = case["name"].as_str().unwrap();
    engine.execute("flat=X(:); c=class(X);").unwrap();
    assert_eq!(text(engine, "c"), case["class"].as_str().unwrap(), "{name}");
    let dims: Vec<usize> = case["size"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap() as usize)
        .collect();
    assert_eq!(engine.array("X").unwrap().dims().as_ref(), dims, "{name}");
    let flat = engine.array("flat").unwrap();
    let values = case["column_major"].as_array().unwrap();
    for (k, v) in values.iter().enumerate() {
        assert_eq!(
            flat.scalar(k, 0).unwrap(),
            matrust::Scalar::Integer(v.as_i64().unwrap() as i128),
            "{name}[{k}]"
        );
    }
    if !values.is_empty() {
        assert_eq!(flat.len(), values.len(), "{name}");
    }
    for sample in case["samples"].as_array().unwrap() {
        let s: Vec<i64> = sample
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_i64().unwrap())
            .collect();
        assert_eq!(
            engine
                .array("X")
                .unwrap()
                .scalar(s[0] as usize, s[1] as usize)
                .unwrap(),
            matrust::Scalar::Integer(s[2] as i128),
            "{name}{s:?}"
        );
    }
}

#[test]
fn independent_metadata_pixels_shapes_and_info_reread() {
    let cases = manifest()["valid"].as_array().unwrap().clone();
    assert!(cases.len() >= 14);
    for case in &cases {
        let name = case["name"].as_str().unwrap();
        let file = format!("{name}.dcm");
        let mut engine = Engine::default();
        call(&mut engine, "dicominfo", &file, "i").unwrap();
        for (key, want) in case["fields"].as_object().unwrap() {
            engine.execute(&format!("q=i.{key};")).unwrap();
            match want.as_str() {
                Some(s) => assert_eq!(text(&engine, "q"), s, "{name}: {key}"),
                None => assert_eq!(
                    number(&engine, "q"),
                    want.as_f64().unwrap(),
                    "{name}: {key}"
                ),
            }
        }
        engine
            .execute("f=i.Filename; has=isfield(i,'PixelData');")
            .unwrap();
        assert_eq!(text(&engine, "f"), fixture(&file).to_string_lossy());
        assert_eq!(
            number(&engine, "has"),
            0.0,
            "{name}: no pixel payload in info"
        );
        call(&mut engine, "dicomread", &file, "X").unwrap();
        check_pixels(&mut engine, case);
        // Caller-edited info fields never reach the decoder; Filename is reparsed.
        engine
            .execute("i.Rows=999; i.Columns=1; i.FileSize=1; i.TransferSyntaxUID='1.2.840.10008.1.2.1'; i.BitsStored=1; X=dicomread(i);")
            .unwrap();
        check_pixels(&mut engine, case);
    }
}

#[test]
fn malformed_encapsulation_and_segments_reject_without_partial_output() {
    let m = manifest();
    let invalid = m["invalid"].as_object().unwrap();
    let info_invalid = m["info_invalid"].as_object().unwrap();
    assert!(invalid.len() >= 30);
    for (name, expected) in invalid {
        let file = format!("{name}.dcm");
        let mut engine = Engine::default();
        let error = call(&mut engine, "dicomread", &file, "X").unwrap_err();
        assert!(
            error.contains(expected.as_str().unwrap()),
            "dicomread {name}: {error}"
        );
        assert!(engine.array("X").is_none(), "{name}");
        let info = call(&mut engine, "dicominfo", &file, "i");
        match info_invalid.get(name) {
            Some(want) => {
                let error = info.unwrap_err();
                assert!(
                    error.contains(want.as_str().unwrap()),
                    "dicominfo {name}: {error}"
                );
            }
            // Metadata-only reads never decode fragment payloads.
            None => info.unwrap_or_else(|e| panic!("dicominfo {name}: {e}")),
        }
    }
}

#[test]
fn envelope_only_rle_file_is_metadata_but_has_no_pixels() {
    // Formerly the unsupported-RLE dicominfo case; now a valid metadata read.
    let mut engine = Engine::default();
    call(&mut engine, "dicominfo", "bad_rle_uid.dcm", "i").unwrap();
    engine.execute("u=i.TransferSyntaxUID;").unwrap();
    assert_eq!(text(&engine, "u"), "1.2.840.10008.1.2.5");
    let error = call(&mut engine, "dicomread", "bad_rle_uid.dcm", "X").unwrap_err();
    assert!(error.contains("missing required Rows"), "{error}");
    // Unregistered encapsulated syntaxes stay explicitly unsupported.
    let error = call(&mut engine, "dicomread", "pixels_bad_compressed.dcm", "X").unwrap_err();
    assert!(error.contains("1.2.840.10008.1.2.4.50"), "{error}");
}

#[test]
fn noop_work_and_low_memory_admission() {
    let noop = format!("{}.dcm", manifest()["noop_work"].as_str().unwrap());
    let mut engine = Engine::default();
    call(&mut engine, "dicomread", &noop, "X").unwrap();
    engine.execute("s=sum(double(X(:)));").unwrap();
    assert_eq!(number(&engine, "s"), 14.0);
    // 256 KiB of -128 no-ops are charged as compressed work before decoding.
    let mut limited = Engine::with_limits(matrust::engine::Limits {
        evaluation_units: Some(20_000),
        ..Default::default()
    });
    call(&mut limited, "dicominfo", &noop, "i").unwrap();
    let error = call(&mut limited, "dicomread", &noop, "X").unwrap_err();
    assert!(
        error.contains("budget") || error.contains("limit"),
        "{error}"
    );
    assert!(limited.array("X").is_none());
    // 512x1025 uint16: metadata fits; decoded planes + typed output + copy do not.
    let mut limited = Engine::with_limits(matrust::engine::Limits {
        memory_bytes: 3 * 1024 * 1024,
        ..Default::default()
    });
    call(&mut limited, "dicominfo", "rle_large.dcm", "i").unwrap();
    let error = call(&mut limited, "dicomread", "rle_large.dcm", "X").unwrap_err();
    assert!(
        error.contains("decoded image") && error.contains("budget"),
        "{error}"
    );
    assert!(limited.array("X").is_none());
    let mut roomy = Engine::with_limits(matrust::engine::Limits {
        memory_bytes: 5 * 1024 * 1024,
        ..Default::default()
    });
    call(&mut roomy, "dicomread", "rle_large.dcm", "X").unwrap();
}

#[test]
#[ignore = "manual optimized RLE timing with 1 and N caller workers"]
fn rle_release_timing() {
    let workers: usize = std::env::var("DICOM_BENCH_WORKERS")
        .unwrap_or_else(|_| "1".into())
        .parse()
        .unwrap();
    rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .unwrap()
        .install(|| {
            for function in ["dicominfo", "dicomread"] {
                let mut engine = Engine::default();
                call(&mut engine, function, "rle_large.dcm", "X").unwrap();
                let start = std::time::Instant::now();
                for _ in 0..100 {
                    call(&mut engine, function, "rle_large.dcm", "X").unwrap();
                }
                eprintln!(
                    "DICOM RLE: function={function}, workers={workers}, iterations=100, decoded_pixel_bytes=1049600 elapsed_ms={:.3}",
                    start.elapsed().as_secs_f64() * 1000.
                );
            }
        });
}

#[test]
fn minimal_replicate_segment_decodes_with_overflow_checks() {
    // PS3.5 G.3.1: ff 07 repeats 07 exactly twice.
    let mut engine = Engine::default();
    call(&mut engine, "dicomread", "rle_minimal_replicate.dcm", "X").unwrap();
    let x = engine.array("X").unwrap();
    assert_eq!(x.dims().as_ref(), [1, 2]);
    assert_eq!(x.scalar(0, 0).unwrap(), matrust::Scalar::Integer(7));
    assert_eq!(x.scalar(0, 1).unwrap(), matrust::Scalar::Integer(7));
}
