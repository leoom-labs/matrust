use matrust::simulation::{
    MAX_LAYOUT_COORDINATE, Model,
    document::{ModelFile, SaveOutcome},
};
use std::sync::atomic::AtomicBool;

fn positions(model: &Model) -> Vec<[f32; 2]> {
    (0..model.nodes.len())
        .map(|i| [24. + i as f32 * 170., 40. + i as f32 * 16.])
        .collect()
}

#[test]
fn model_layout_versioned_roundtrip_keeps_legacy_encoding_and_simulation_bits() {
    let mut model = Model::default();
    let original = serde_json::to_string(&model).unwrap();
    assert!(!original.contains("positions"));
    let restored: Model = serde_json::from_str(&original).unwrap();
    assert!(model.same_storage(&restored));
    let before = model.run(&AtomicBool::new(false)).unwrap();
    model.set_layout(positions(&model)).unwrap();
    assert_eq!(model.version, 2);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("positioned.mrsim");
    model.save(&path).unwrap();
    let restored = Model::load(&path).unwrap();
    assert!(model.same_storage(&restored));
    let after = restored.run(&AtomicBool::new(false)).unwrap();
    assert_eq!(
        bytemuck::cast_slice::<f64, u64>(before.time.data()),
        bytemuck::cast_slice::<f64, u64>(after.time.data())
    );
    for (a, b) in before.traces.iter().zip(after.traces) {
        assert_eq!(a.name, b.name);
        assert_eq!(
            bytemuck::cast_slice::<f64, u64>(a.values.data()),
            bytemuck::cast_slice::<f64, u64>(b.values.data())
        );
    }
    model.set_layout(vec![]).unwrap();
    assert_eq!(serde_json::to_string(&model).unwrap(), original);
}

#[test]
fn model_layout_rejects_bad_versions_bounds_and_counts_before_mutation_or_save() {
    let model = Model::default();
    let mut candidate = model.clone();
    for bad in [-1., f32::NAN, f32::INFINITY, MAX_LAYOUT_COORDINATE + 1.] {
        let mut layout = positions(&model);
        layout[0][0] = bad;
        assert!(candidate.set_layout(layout).is_err());
        assert!(candidate.same_storage(&model));
    }
    assert!(candidate.set_layout(vec![[0., 0.]; 1]).is_err());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keep.mrsim");
    std::fs::write(&path, b"keep").unwrap();
    for version in [1, 2, 3] {
        candidate = model.clone();
        candidate.version = version;
        candidate.positions = if version == 1 {
            positions(&model)
        } else {
            vec![]
        };
        assert!(candidate.validate_layout().is_err());
        assert!(candidate.save(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"keep");
    }
    candidate = model.clone();
    let mut layout = positions(&model);
    layout[0] = [-0., MAX_LAYOUT_COORDINATE];
    candidate.set_layout(layout).unwrap();
    let mut other = candidate.clone();
    other.positions[0][0] = 0.;
    assert!(!candidate.same_storage(&other));
}

#[test]
fn model_layout_only_disk_edits_are_detected_as_conflicts() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("external.mrsim");
    Model::default().save(&path).unwrap();
    let (mine, mut file) = ModelFile::open(&path).unwrap();
    let mut external = mine.clone();
    external.set_layout(positions(&external)).unwrap();
    external.save(&path).unwrap();
    assert!(file.external_change().unwrap().is_some());
    assert!(matches!(
        file.try_save(&mine, &path, None).unwrap(),
        SaveOutcome::Conflict(_)
    ));
    assert!(Model::load(&path).unwrap().same_storage(&external));
    assert!(!file.dirty(&mine));
}
