//! Exact lossless oracle: original encoder input arrays, independently decoded
//! with libjpeg; regeneration and package provenance in tool/refs/dicom.
use matrust::engine::Engine;
use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/dicom")
        .join(format!("{name}.dcm"))
}

#[test]
fn jpeg_lossless_public_baseline() {
    let mut engine = Engine::default();
    let path = fixture("jl_public");
    engine
        .execute(&format!("X=dicomread('{}');", path.display()))
        .unwrap();
    assert_eq!(engine.array("X").unwrap().class(), "uint16");
    assert_eq!(engine.array("X").unwrap().dims().as_ref(), [8, 16]);
    compare(&mut engine, &manifest()["valid"][0]);
}

fn manifest() -> serde_json::Value {
    serde_json::from_slice(include_bytes!("fixtures/dicom/jpeg_lossless.json")).unwrap()
}

fn compare(engine: &mut Engine, case: &serde_json::Value) {
    let name = case["name"].as_str().unwrap();
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
            panic!("{name}[{k}]: not an integer");
        };
        assert_eq!(got, i128::from(value.as_i64().unwrap()), "{name}[{k}]");
    }
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
fn jpeg_lossless_all_precisions_predictors_classes_and_layouts() {
    for case in manifest()["valid"].as_array().unwrap() {
        if case["timing_only"].as_bool() == Some(true) {
            continue;
        }
        let name = case["name"].as_str().unwrap();
        let mut engine = Engine::default();
        call(&mut engine, "dicominfo", name, "i").unwrap();
        call(&mut engine, "dicomread", name, "X").unwrap_or_else(|e| panic!("{name}: {e}"));
        compare(&mut engine, case);
        engine.execute("i.Rows=999; i.PhotometricInterpretation='IGNORED'; Y=dicomread(i); same=isequal(X,Y);").unwrap();
        assert_eq!(
            engine
                .array("same")
                .unwrap()
                .scalar(0, 0)
                .unwrap()
                .into_real()
                .unwrap(),
            1.0,
            "{name}"
        );
    }
}

#[test]
fn jpeg_lossless_malformed_never_returns_a_partial_image() {
    let m = manifest();
    for (name, reason) in m["invalid"].as_object().unwrap() {
        let mut engine = Engine::default();
        // Metadata-only access succeeds without inspecting the codestream.
        call(&mut engine, "dicominfo", name, "i").unwrap_or_else(|e| panic!("info {name}: {e}"));
        let error = call(&mut engine, "dicomread", name, "X").unwrap_err();
        let expected = match reason.as_str().unwrap() {
            "truncated" => "entropy",
            "wrong_sof" => "SOF3",
            "precision" => "BitsStored",
            "components" => "SamplesPerPixel",
            "dimensions" => "Rows x Columns",
            "predictor" => "requires predictor 1",
            "oversize" => "limit",
            "trailing" => "after EOI",
            "photo" => "PhotometricInterpretation",
            "point" => "point transform",
            "eot" => "Extended Offset Table",
            "restart" => "restart intervals",
            _ => panic!("missing reason {reason}"),
        };
        assert!(
            error.contains(expected),
            "{name}: expected {expected}, got {error}"
        );
        assert!(engine.array("X").is_none(), "{name}");
    }
}

#[test]
fn jpeg_lossless_fixture_hashes() {
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
fn jpeg_lossless_admission_and_work_limits() {
    let mut engine = Engine::with_limits(matrust::engine::Limits {
        memory_bytes: 256 * 1024,
        ..Default::default()
    });
    call(&mut engine, "dicominfo", "jl_public", "i").unwrap();
    let error = call(&mut engine, "dicomread", "jl_public", "X").unwrap_err();
    assert!(error.contains("budget"), "{error}");
    assert!(engine.array("X").is_none());
    let mut engine = Engine::with_limits(matrust::engine::Limits {
        evaluation_units: Some(128),
        ..Default::default()
    });
    let error = call(&mut engine, "dicomread", "jl_public", "X").unwrap_err();
    assert!(
        error.contains("budget") || error.contains("limit"),
        "{error}"
    );
    assert!(engine.array("X").is_none());
}

#[test]
#[ignore = "manual gate-profile timing with 1 and N caller workers"]
fn jpeg_lossless_release_timing() {
    let workers: usize = std::env::var("DICOM_BENCH_WORKERS")
        .unwrap_or_else(|_| "1".into())
        .parse()
        .unwrap();
    rayon::ThreadPoolBuilder::new().num_threads(workers).build().unwrap().install(|| {
        for function in ["dicominfo", "dicomread"] {
            let mut engine = Engine::default();
            call(&mut engine, function, "jl_large", "X").unwrap();
            let start = std::time::Instant::now();
            for _ in 0..20 {
                call(&mut engine, function, "jl_large", "X").unwrap();
            }
            eprintln!("JPEG Lossless {function}: caller_workers={workers}, iterations=20, 512x1024 uint16, elapsed_ms={:.3}", start.elapsed().as_secs_f64()*1000.);
        }
    });
}
