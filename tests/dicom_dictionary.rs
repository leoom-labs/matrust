//! Full public dictionary and public metadata regressions.
use matrust::Engine;

#[test]
fn dicominfo_exposes_previously_unknown_standard_field() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/dicom/write_current/u8_0.dcm");
    let mut engine = Engine::default();
    engine
        .execute(&format!(
            "i=dicominfo('{}'); present=isfield(i,'ConversionType');",
            path.to_string_lossy().replace('\'', "''")
        ))
        .unwrap();
    assert_eq!(
        engine
            .array("present")
            .unwrap()
            .scalar(0, 0)
            .unwrap()
            .into_real()
            .unwrap(),
        1.,
        "ConversionType field absent"
    );
    engine.execute("value=i.ConversionType;").unwrap();
    let matrust::Array::Character(value) = engine.array("value").unwrap() else {
        panic!("expected text")
    };
    assert_eq!(
        value
            .to_utf8(&std::sync::atomic::AtomicBool::new(false))
            .unwrap(),
        "WSD"
    );
}

// Exercise the same immutable implementation without exposing a public Rust API.
#[path = "../src/engine/dicom/dictionary.rs"]
mod dictionary;

fn fixture(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/dicom")
        .join(name)
}
fn read_info(
    engine: &mut Engine,
    path: &std::path::Path,
) -> matrust::Result<matrust::engine::Execution> {
    engine.execute(&format!(
        "i=dicominfo('{}');",
        path.to_string_lossy().replace('\'', "''")
    ))
}
fn scalar(engine: &Engine, name: &str) -> f64 {
    engine
        .array(name)
        .unwrap()
        .scalar(0, 0)
        .unwrap()
        .into_real()
        .unwrap()
}

#[test]
fn generated_counts_sorted_unique_and_keyword_round_trips() {
    let generated = include_str!("../src/engine/dicom/dictionary_data.rs");
    let counts = generated
        .lines()
        .find(|line| line.starts_with("// Counts:"))
        .unwrap();
    assert_eq!(
        counts,
        format!(
            "// Counts: exact={}, masks={}, skipped=10",
            dictionary::EXACT.len(),
            dictionary::REPEATERS.len()
        )
    );
    assert_eq!(
        (dictionary::EXACT.len(), dictionary::REPEATERS.len()),
        (5211, 88)
    );
    assert!(
        dictionary::EXACT
            .windows(2)
            .all(|w| w[0].value < w[1].value)
    );
    assert!(
        dictionary::REPEATERS
            .windows(2)
            .all(|w| (w[0].value, w[0].mask) < (w[1].value, w[1].mask))
    );
    let entries: Vec<_> = dictionary::EXACT
        .iter()
        .chain(dictionary::REPEATERS)
        .collect();
    assert_eq!(dictionary::KEYWORDS.len(), entries.len());
    let keywords: Vec<_> = dictionary::KEYWORDS
        .iter()
        .map(|i| entries[usize::from(*i)].keyword)
        .collect();
    assert!(keywords.windows(2).all(|w| w[0] < w[1]));
    for entry in entries {
        assert_eq!(entry.value & entry.mask, entry.value);
        assert!((1..=3).contains(&entry.vrs.len()));
        assert!(!entry.vm.is_empty() && !entry.name.is_empty());
        assert!(std::ptr::eq(
            dictionary::by_keyword(entry.keyword).unwrap(),
            entry
        ));
        let found = dictionary::lookup(entry.value).unwrap();
        if entry.mask != u32::MAX && found.mask == u32::MAX {
            // Nth-order coefficient repeaters overlap exact transform tags
            // at their all-zero wildcard; exact tags take precedence.
            assert_eq!(found.value, entry.value);
        } else {
            assert!(std::ptr::eq(found, entry), "{}", entry.keyword);
        }
    }
    assert!(dictionary::EXACT.iter().all(|e| e.mask == u32::MAX));
    assert!(dictionary::REPEATERS.iter().all(|e| e.mask != u32::MAX));
    assert!(dictionary::by_keyword("patientname").is_none());
    for tag in [
        0x0000_0000,
        0x0011_0010,
        0x5001_0005,
        0x6001_0010,
        0x1234_5678,
    ] {
        assert!(dictionary::lookup(tag).is_none(), "{tag:08X}");
    }
}

#[test]
fn public_registry_spots_retired_multivr_and_arbitrary_masks() {
    // Published NEMA PS3.6 2026d tables 6-1, 7-1, 8-1 (SOURCES.md).
    for (tag, keyword, vr) in [
        (0x0002_0010, "TransferSyntaxUID", *b"UI"),
        (0x0002_0016, "SourceApplicationEntityTitle", *b"AE"),
        (0x0004_1130, "FileSetID", *b"CS"),
        (0x0008_0001, "LengthToEnd", *b"UL"),
        (0x0008_0064, "ConversionType", *b"CS"),
        (0x0008_0090, "ReferringPhysicianName", *b"PN"),
        (0x0010_0010, "PatientName", *b"PN"),
        (0x0010_1010, "PatientAge", *b"AS"),
        (0x0010_2160, "EthnicGroup", *b"SH"),
        (0x0018_0050, "SliceThickness", *b"DS"),
        (0x0018_0080, "RepetitionTime", *b"DS"),
        (0x0018_9810, "ZeroVelocityPixelValue", *b"US"),
        (0x0020_0010, "StudyID", *b"SH"),
        (0x0020_31AB, "SourceImageIDs", *b"CS"),
        (0x0028_0010, "Rows", *b"US"),
        (0x0028_0400, "TransformLabel", *b"LO"),
        (0x0028_0410, "RowsForNthOrderCoefficients", *b"US"),
        (0x0028_0106, "SmallestImagePixelValue", *b"US"),
        (0x0028_1200, "GrayLookupTableData", *b"US"),
        (0x0040_0244, "PerformedProcedureStepStartDate", *b"DA"),
        (0x0040_9216, "RealWorldValueFirstValueMapped", *b"US"),
        (0x3006_0020, "StructureSetROISequence", *b"SQ"),
        (0x300A_00B0, "BeamSequence", *b"SQ"),
        (0x300A_011E, "GantryAngle", *b"DS"),
        (0x7FE0_0001, "ExtendedOffsetTable", *b"OV"),
        (0x7FE0_0008, "FloatPixelData", *b"OF"),
        (0x7FE0_0010, "PixelData", *b"OB"),
        (0x5002_0005, "CurveDimensions", *b"US"),
        (0x6002_0010, "OverlayRows", *b"US"),
        (0x1000_1230, "EscapeTriplet", *b"US"),
        (0x7F10_0010, "VariablePixelData", *b"OB"),
    ] {
        let entry = dictionary::lookup(tag).unwrap();
        assert_eq!((entry.keyword, entry.vrs[0]), (keyword, vr), "{tag:08X}");
    }
    assert!(dictionary::lookup(0x0008_0001).unwrap().retired);
    assert_eq!(
        dictionary::lookup(0x0028_0106).unwrap().vrs,
        [*b"US", *b"SS"]
    );
    let triple = dictionary::lookup(0x0028_1200).unwrap();
    assert_eq!(triple.vrs, [*b"US", *b"SS", *b"OW"]);
    assert_eq!(dictionary::implicit_vr(triple, triple.value, 1), *b"US");
    assert_eq!(
        dictionary::implicit_vr(dictionary::lookup(0x7FE0_0010).unwrap(), 0x7FE0_0010, 0),
        *b"OW"
    );
}

#[test]
fn implicit_signedness_and_explicit_members_match_pydicom() {
    let expected: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/dicom/dictionary.json")).unwrap();
    for (fixture_name, fields) in expected.as_object().unwrap() {
        let mut engine = Engine::default();
        read_info(&mut engine, &fixture(&format!("{fixture_name}.dcm"))).unwrap();
        for (field, value) in fields.as_object().unwrap() {
            engine.execute(&format!("q=i.{field};")).unwrap();
            assert_eq!(
                scalar(&engine, "q"),
                value.as_f64().unwrap(),
                "{fixture_name} {field}"
            );
            assert_eq!(
                engine.array("q").unwrap().class(),
                if value.as_i64().unwrap() < 0 {
                    "int16"
                } else {
                    "uint16"
                },
                "{fixture_name} {field}"
            );
        }
    }
}

fn envelope(implicit: bool) -> Vec<u8> {
    let bytes: &[u8] = if implicit {
        include_bytes!("fixtures/dicom/dictionary_implicit_0.dcm")
    } else {
        include_bytes!("fixtures/dicom/baseline.dcm")
    };
    let length = u32::from_le_bytes(bytes[140..144].try_into().unwrap()) as usize;
    bytes[..144 + length].to_vec()
}
fn element(out: &mut Vec<u8>, tag: u32, vr: &[u8; 2], bytes: &[u8], implicit: bool) {
    assert!(bytes.len().is_multiple_of(2));
    out.extend_from_slice(&((tag >> 16) as u16).to_le_bytes());
    out.extend_from_slice(&(tag as u16).to_le_bytes());
    if implicit {
        out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    } else {
        out.extend_from_slice(vr);
        if matches!(vr, b"OB" | b"OW" | b"UN") {
            out.extend_from_slice(&[0, 0]);
            out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        } else {
            out.extend_from_slice(&u16::try_from(bytes.len()).unwrap().to_le_bytes());
        }
    }
    out.extend_from_slice(bytes);
}

#[test]
fn explicit_vr_rejects_nonmember_preserves_un_and_accepts_ob_ow() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vr.dcm");
    for (tag, vr, bytes, valid) in [
        (0x0028_0106, *b"US", vec![255, 255], true),
        (0x0028_0106, *b"SS", vec![255, 255], true),
        (0x0028_0106, *b"UL", vec![0; 4], false),
        (0x0028_0106, *b"UN", vec![0; 2], true),
        (0x0028_1200, *b"US", vec![0; 2], true),
        (0x0028_1200, *b"SS", vec![0; 2], true),
        (0x0028_1200, *b"OW", vec![0; 2], true),
        (0x0010_0020, *b"US", vec![0; 2], false), // existing single-VR policy
        (0x6000_3000, *b"OB", vec![0; 2], true),
        (0x6000_3000, *b"OW", vec![0; 2], true),
    ] {
        let mut data = envelope(false);
        element(&mut data, tag, &vr, &bytes, false);
        std::fs::write(&path, data).unwrap();
        let mut engine = Engine::default();
        let result = read_info(&mut engine, &path);
        if valid {
            result.unwrap();
            if vr == *b"UN" {
                engine
                    .execute("present=isfield(i,'SmallestImagePixelValue');")
                    .unwrap();
                assert_eq!(scalar(&engine, "present"), 0.);
            }
        } else {
            let error = result.unwrap_err();
            assert!(error.contains("explicit VR disagrees"), "{error}");
        }
    }
}

#[test]
fn repeating_fields_are_distinct_and_private_groups_stay_absent() {
    let mut data = envelope(false);
    for (tag, vr, value) in [
        (0x0020_3100, *b"CS", b"A ".as_slice()),
        (0x0020_31AB, *b"CS", b"B "),
        (0x5000_0005, *b"US", &[2, 0]),
        (0x5002_0005, *b"US", &[3, 0]),
        (0x6000_0010, *b"US", &[4, 0]),
        (0x6001_0010, *b"US", &[99, 0]),
        (0x6002_0010, *b"US", &[5, 0]),
    ] {
        element(&mut data, tag, &vr, value, false);
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("repeaters.dcm");
    std::fs::write(&path, data).unwrap();
    let mut engine = Engine::default();
    read_info(&mut engine, &path).unwrap();
    for (field, want) in [
        ("CurveDimensions_0", 2.),
        ("CurveDimensions_2", 3.),
        ("OverlayRows_0", 4.),
        ("OverlayRows_2", 5.),
    ] {
        engine.execute(&format!("q=i.{field};")).unwrap();
        assert_eq!(scalar(&engine, "q"), want);
    }
    engine.execute("a=isfield(i,'SourceImageIDs'); b=isfield(i,'SourceImageIDs_AB'); p=isfield(i,'OverlayRows_1');").unwrap();
    assert_eq!(
        (
            scalar(&engine, "a"),
            scalar(&engine, "b"),
            scalar(&engine, "p")
        ),
        (1., 1., 0.)
    );
    assert_eq!(
        dictionary::field_name(dictionary::lookup(0x6002_0010).unwrap(), 0x6002_0010),
        "OverlayRows_2"
    );
}

#[test]
fn newly_known_large_values_are_budgeted_and_cancelled() {
    // 16 distinct OverlayData values, all unknown to the old hand table.
    // Input ~1 MiB fits the file limit; materialization's 16x charge exceeds
    // the 16 MiB parser cap, even with the default 256 MiB engine workspace.
    let mut data = envelope(false);
    for group in (0x6000..0x6020).step_by(2) {
        element(
            &mut data,
            (group << 16) | 0x3000,
            b"OW",
            &vec![0; 65536],
            false,
        );
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("large-new-fields.dcm");
    std::fs::write(&path, data).unwrap();
    let mut engine = Engine::default();
    let error = read_info(&mut engine, &path).unwrap_err();
    assert!(error.contains("memory/output budget"), "{error}");
    assert!(engine.array("i").is_none());
    engine
        .cancellation()
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let error = read_info(&mut engine, &path).unwrap_err();
    assert!(error.contains("cancelled"), "{error}");
    // Deterministic mid-parse cancellation is additionally covered by the
    // watched-reader unit test in parser.rs using the same newly known tags.
}

#[test]
fn newly_known_value_limit_and_small_materialization() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("new-value.dcm");
    let mut data = envelope(false);
    element(&mut data, 0x0042_0011, b"OB", &[1, 2, 3, 4], false);
    std::fs::write(&path, data).unwrap();
    let mut engine = Engine::default();
    read_info(&mut engine, &path).unwrap();
    engine.execute("v=i.EncapsulatedDocument;").unwrap();
    let value = engine.array("v").unwrap();
    assert_eq!(value.class(), "uint8");
    assert_eq!(value.len(), 4);
    for index in 0..4 {
        assert_eq!(
            value.scalar(index, 0).unwrap().into_real().unwrap(),
            (index + 1) as f64
        );
    }
    let mut data = envelope(false);
    element(
        &mut data,
        0x0042_0011,
        b"OB",
        &vec![0; 1024 * 1024 + 2],
        false,
    );
    std::fs::write(&path, data).unwrap();
    let error = read_info(&mut engine, &path).unwrap_err();
    assert!(error.contains("1048576-byte limit"), "{error}");
}

#[test]
fn keyword_index_does_not_widen_writer_allowlist() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("forbidden.dcm");
    let mut engine = Engine::default();
    engine.execute(&format!(
        "m=struct('SliceThickness',1); dicomwrite(uint8([1 2]),'{}',m); i=dicominfo('{}'); extra=isfield(i,'SliceThickness');",
        path.display(), path.display()
    )).unwrap();
    assert_eq!(scalar(&engine, "extra"), 0.);
    assert!(path.is_file());
}

#[test]
fn implicit_missing_or_empty_context_uses_first_vr() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("implicit-context.dcm");
    for empty in [false, true] {
        let mut data = envelope(true);
        if empty {
            element(&mut data, 0x0028_0103, b"US", &[], true);
        }
        element(&mut data, 0x0028_0120, b"US", &[255, 255], true);
        std::fs::write(&path, data).unwrap();
        let mut engine = Engine::default();
        read_info(&mut engine, &path).unwrap();
        engine.execute("v=i.PixelPaddingValue;").unwrap();
        assert_eq!(scalar(&engine, "v"), 65535.);
        assert_eq!(engine.array("v").unwrap().class(), "uint16");
        if empty {
            engine.execute("v=isempty(i.PixelRepresentation);").unwrap();
            assert_eq!(scalar(&engine, "v"), 1.);
        }
    }
}

#[test]
fn dicomlookup_public_baseline() {
    let mut engine = Engine::default();
    engine.execute("name=dicomlookup('0010','0010');").unwrap();
    let matrust::Array::Character(value) = engine.array("name").unwrap() else {
        panic!("expected text")
    };
    assert_eq!(
        value
            .to_utf8(&std::sync::atomic::AtomicBool::new(false))
            .unwrap(),
        "PatientName"
    );
}
