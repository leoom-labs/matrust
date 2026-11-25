//! Independent encoder fixtures: tool/refs/dicom/jpeg2000_fixtures.py.
use matrust::engine::Engine;
use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/dicom")
        .join(format!("{name}.dcm"))
}

#[test]
fn jpeg2000_public_baseline() {
    let mut engine = Engine::default();
    engine
        .execute(&format!(
            "X=dicomread('{}');",
            fixture("j2k_public").display()
        ))
        .unwrap();
    let x = engine.array("X").unwrap();
    assert_eq!(x.class(), "uint16");
    assert_eq!(x.dims().as_ref(), [8, 16]);
    for c in 0..16 {
        for r in 0..8 {
            assert_eq!(
                x.scalar(r, c).unwrap().into_real().unwrap(),
                ((r * 341 + c * 37) % 4096) as f64
            );
        }
    }
}

fn manifest() -> serde_json::Value {
    serde_json::from_slice(include_bytes!("fixtures/dicom/jpeg2000.json")).unwrap()
}
fn call(engine: &mut Engine, function: &str, name: &str, output: &str) -> Result<(), String> {
    engine
        .execute(&format!(
            "{output}={function}('{}');",
            fixture(name).display()
        ))
        .map(|_| ())
}

#[test]
fn jpeg2000_exact_oracles_classes_shapes_and_colour() {
    let mut lossy_max = 0i128;
    for case in manifest()["valid"].as_array().unwrap() {
        if case["timing_only"].as_bool() == Some(true) {
            continue;
        }
        let name = case["name"].as_str().unwrap();
        let mut engine = Engine::default();
        call(&mut engine, "dicominfo", name, "i").unwrap();
        call(&mut engine, "dicomread", name, "X").unwrap_or_else(|e| panic!("{name}: {e}"));
        let x = engine.array("X").unwrap();
        assert_eq!(x.class(), case["class"].as_str().unwrap(), "{name}");
        let dims: Vec<usize> = case["size"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap() as usize)
            .collect();
        assert_eq!(x.dims().as_ref(), dims, "{name}");
        engine.execute("flat=X(:);").unwrap();
        let flat = engine.array("flat").unwrap();
        let want = case["column_major"].as_array().unwrap();
        assert_eq!(flat.len(), want.len(), "{name}");
        for (k, value) in want.iter().enumerate() {
            let matrust::Scalar::Integer(got) = flat.scalar(k, 0).unwrap() else {
                panic!("not integer");
            };
            let diff = (got - i128::from(value.as_i64().unwrap())).abs();
            if name.starts_with("j2k_lossy_") {
                lossy_max = lossy_max.max(diff);
            }
            assert_eq!(diff, 0, "{name}[{k}]");
        }
        engine.execute(&format!("photo=strcmp(i.PhotometricInterpretation,'{}'); Y=dicomread(i); same=isequal(X,Y);",case["photo"].as_str().unwrap())).unwrap();
        assert_eq!(
            engine
                .array("photo")
                .unwrap()
                .scalar(0, 0)
                .unwrap()
                .into_real()
                .unwrap(),
            1.0
        );
        assert_eq!(
            engine
                .array("same")
                .unwrap()
                .scalar(0, 0)
                .unwrap()
                .into_real()
                .unwrap(),
            1.0
        );
    }
    eprintln!("JPEG 2000 lossy vs Pillow max absolute difference: {lossy_max}");
}

#[test]
fn jpeg2000_malformed_never_returns_partial_pixels() {
    for (name, reason) in manifest()["invalid"].as_object().unwrap() {
        let mut engine = Engine::default();
        call(&mut engine, "dicominfo", name, "i")
            .unwrap_or_else(|e| panic!("metadata {name}: {e}"));
        let error = call(&mut engine, "dicomread", name, "X")
            .err()
            .unwrap_or_else(|| panic!("{name}: malformed frame succeeded"));
        assert!(error.contains(reason.as_str().unwrap()), "{name}: {error}");
        assert!(engine.array("X").is_none(), "{name}");
    }
}

#[test]
fn jpeg2000_memory_and_work_admission() {
    for limits in [
        matrust::engine::Limits {
            memory_bytes: 256 * 1024,
            ..Default::default()
        },
        matrust::engine::Limits {
            evaluation_units: Some(128),
            ..Default::default()
        },
    ] {
        let metadata = limits.evaluation_units.is_none();
        let mut engine = Engine::with_limits(limits);
        if metadata {
            call(&mut engine, "dicominfo", "j2k_public", "i").unwrap();
        }
        let error = call(&mut engine, "dicomread", "j2k_public", "X").unwrap_err();
        assert!(
            error.contains("budget") || error.contains("limit"),
            "{error}"
        );
        assert!(engine.array("X").is_none());
    }
}

#[test]
fn jpeg2000_other_transfer_syntaxes_stay_explicit() {
    for (name, uid) in manifest()["unsupported"].as_object().unwrap() {
        for function in ["dicominfo", "dicomread"] {
            let mut engine = Engine::default();
            let error = call(&mut engine, function, name, "X").unwrap_err();
            assert!(
                error.contains(uid.as_str().unwrap()) && error.contains("not supported yet"),
                "{name}: {error}"
            );
        }
    }
}

#[test]
fn jpeg2000_fixture_hashes() {
    use sha2::Digest;
    for (name, want) in manifest()["sha256"].as_object().unwrap() {
        let bytes = std::fs::read(fixture(name.strip_suffix(".dcm").unwrap())).unwrap();
        let hash: String = sha2::Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(hash, want.as_str().unwrap(), "{name}");
    }
}

#[test]
#[ignore = "manual debug/gate timing, 1 and N caller workers"]
fn jpeg2000_timing() {
    let workers: usize = std::env::var("DICOM_BENCH_WORKERS")
        .unwrap_or_else(|_| "1".into())
        .parse()
        .unwrap();
    rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .unwrap()
        .install(|| {
            let mut engine = Engine::default();
            call(&mut engine, "dicomread", "j2k_large", "X").unwrap();
            let start = std::time::Instant::now();
            for _ in 0..5 {
                call(&mut engine, "dicomread", "j2k_large", "X").unwrap();
            }
            eprintln!(
                "JPEG2000 512x512 uint16 workers={workers}, iterations=5, per_frame_ms={:.3}",
                start.elapsed().as_secs_f64() * 200.
            );
        });
}
