use matrust::{
    plot::Canvas,
    session::{ExportFormat, ExportSettings},
};
#[test]
fn export_preferences_roundtrip_and_normalize_invalid_optional_values() {
    let settings = ExportSettings {
        format: ExportFormat::Pdf,
        width: 1920,
        height: 1080,
        vector: Canvas {
            width: 595.2756,
            height: 841.8898,
        },
    };
    assert_eq!(
        serde_json::from_slice::<ExportSettings>(&serde_json::to_vec(&settings).unwrap()).unwrap(),
        settings
    );
    let mut invalid = settings;
    invalid.width = u32::MAX;
    invalid.vector.height = f32::NAN;
    invalid.normalize();
    assert_eq!((invalid.width, invalid.height), (1440, 840));
    assert_eq!(invalid.vector, Canvas::default());
    assert_eq!(invalid.format, ExportFormat::Pdf);
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/ui/wide-session.json");
    let mut json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    // An older session has no export preferences. A malformed preference must
    // not prevent recovering its documents and workspace references.
    json.as_object_mut().unwrap().remove("export");
    let old: matrust::session::Session = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(old.export, ExportSettings::default());
    json["export"] = serde_json::json!({"format":"pdf","width":1920,"height":1080,"vector":{"width":-1,"height":10}});
    let mut session: matrust::session::Session = serde_json::from_value(json.clone()).unwrap();
    session.validate().unwrap();
    assert_eq!(session.export.vector, Canvas::default());
    assert_eq!(session.export.width, 1920);
    json["export"] = serde_json::json!({"format":"future"});
    let recovered: matrust::session::Session = serde_json::from_value(json).unwrap();
    assert_eq!(recovered.export, ExportSettings::default());
    assert_eq!(recovered.documents.len(), old.documents.len());
    assert!(recovered.workspace_manifest.is_some());
}

#[test]
fn eps_preferences_are_vector_points_and_recover_without_hiding_legacy_work() {
    let settings = ExportSettings {
        format: ExportFormat::Eps,
        vector: Canvas {
            width: 612.,
            height: 792.,
        },
        ..Default::default()
    };
    assert!(settings.format.vector() && !settings.format.raster());
    assert_eq!(settings.format.label(), "EPS (color vector)");
    assert_eq!(settings.format.extensions(), ["eps"]);
    assert_eq!(settings.format.file_name(7), "figure-7.eps");
    let value = serde_json::to_value(settings).unwrap();
    assert_eq!(value["format"], "eps");
    assert_eq!(
        serde_json::from_value::<ExportSettings>(value.clone()).unwrap(),
        settings
    );
    let mut json: serde_json::Value =
        serde_json::from_slice(include_bytes!("../tests/fixtures/ui/wide-session.json")).unwrap();
    json.as_object_mut().unwrap().remove("export");
    let old: matrust::session::Session = serde_json::from_value(json.clone()).unwrap();
    json["export"] = value;
    let recovered: matrust::session::Session = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(recovered.export, settings);
    assert_eq!(recovered.documents.len(), old.documents.len());
    assert_eq!(recovered.active, old.active);
    assert!(recovered.workspace_manifest.is_some());
    json["export"]["format"] = serde_json::json!("unknown_future_format");
    let fallback: matrust::session::Session = serde_json::from_value(json).unwrap();
    assert_eq!(fallback.export, ExportSettings::default());
    assert_eq!(fallback.documents.len(), old.documents.len());
    assert!(fallback.workspace_manifest.is_some());
}
