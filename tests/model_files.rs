use matrust::simulation::{
    Block, Model, Node,
    document::{DiskVersion, ModelFile, SaveOutcome},
};
use std::fs;

#[test]
fn model_comparison_covers_all_storage_fields_and_float_bits() {
    let blocks = vec![
        Block::Constant { value: 0. },
        Block::Sine {
            amplitude: 1.,
            frequency_hz: 2.,
            phase: 3.,
        },
        Block::Step {
            time: 1.,
            before: 2.,
            after: 3.,
        },
        Block::Gain { gain: 2. },
        Block::Sum,
        Block::UnitDelay { initial: 0. },
        Block::Filter {
            b: vec![1., 2.],
            a: vec![1., 0.],
        },
        Block::SosFilter {
            sections: vec![[1., 2., 3., 4., 5., 6.]],
        },
        Block::Scope,
    ];
    let baseline = Model {
        version: 1,
        positions: Vec::new(),
        sample_time: 0.001,
        steps: 100,
        nodes: blocks
            .into_iter()
            .map(|block| Node {
                name: "test".into(),
                block,
                inputs: vec![0],
            })
            .collect(),
    };
    let bytes = serde_json::to_vec(&baseline).unwrap();
    assert!(baseline.same_storage(&baseline.clone()));
    let mut cases = Vec::new();
    let mut m = baseline.clone();
    m.version = 2;
    cases.push(m);
    let mut m = baseline.clone();
    m.steps += 1;
    cases.push(m);
    let mut m = baseline.clone();
    m.sample_time = 0.002;
    cases.push(m);
    let mut m = baseline.clone();
    m.nodes.pop();
    cases.push(m);
    for i in 0..baseline.nodes.len() {
        let mut m = baseline.clone();
        m.nodes[i].name.push('x');
        cases.push(m);
        let mut m = baseline.clone();
        m.nodes[i].inputs.push(1);
        cases.push(m);
        let fields = match &baseline.nodes[i].block {
            Block::Sine { .. } | Block::Step { .. } => 3,
            Block::Filter { .. } => 4,
            Block::SosFilter { .. } => 6,
            _ => 1,
        };
        for field in 0..fields {
            let mut m = baseline.clone();
            match &mut m.nodes[i].block {
                Block::Constant { value }
                | Block::Gain { gain: value }
                | Block::UnitDelay { initial: value } => *value = -0.,
                Block::Sine {
                    amplitude,
                    frequency_hz,
                    phase,
                } => *[amplitude, frequency_hz, phase][field] = -0.,
                Block::Step {
                    time,
                    before,
                    after,
                } => *[time, before, after][field] = -0.,
                Block::Filter { b, a } => {
                    if field < 2 {
                        b[field] = -0.
                    } else {
                        a[field - 2] = -0.
                    }
                }
                Block::SosFilter { sections } => sections[0][field] = -0.,
                block => *block = Block::Constant { value: 4. },
            }
            cases.push(m);
        }
    }
    for m in cases {
        assert_ne!(serde_json::to_vec(&m).unwrap(), bytes);
        assert!(!baseline.same_storage(&m));
    }
    let mut m = baseline.clone();
    m.nodes[0].block = Block::Constant {
        value: f64::from_bits(0x7ff8_0000_0000_1234),
    };
    assert!(m.same_storage(&m.clone()));
    assert!(m.validate_storage().is_err());
}

#[test]
fn model_save_checks_semantic_changes_and_stale_confirmation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("model.mrsim");
    Model::default().save(&path).unwrap();
    let (mut mine, mut file) = ModelFile::open(&path).unwrap();
    mine.steps = 321;
    let mut disk = Model {
        steps: 654,
        ..Model::default()
    };
    disk.save(&path).unwrap();
    let SaveOutcome::Conflict(first) = file.try_save(&mine, &path, None).unwrap() else {
        panic!("conflict expected")
    };
    assert!(file.dirty(&mine));
    disk.steps = 987;
    disk.save(&path).unwrap();
    let SaveOutcome::Conflict(second) = file.try_save(&mine, &path, Some(&first)).unwrap() else {
        panic!("stale confirmation")
    };
    assert_eq!(Model::load(&path).unwrap().steps, 987);
    assert!(matches!(
        file.try_save(&mine, &path, Some(&second)).unwrap(),
        SaveOutcome::Saved
    ));
    assert!(!file.dirty(&mine));
    // Formatting alone does not create a conflict.
    fs::write(&path, serde_json::to_vec(&mine).unwrap()).unwrap();
    assert!(file.external_change().unwrap().is_none());
    assert!(matches!(
        file.try_save(&mine, &path, None).unwrap(),
        SaveOutcome::Saved
    ));
}

#[test]
fn model_deletion_failures_save_as_and_recovery_retain_the_baseline() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("model.mrsim");
    Model::default().save(&path).unwrap();
    let (mut mine, original) = ModelFile::open(&path).unwrap();
    mine.steps = 777;
    let mut file = ModelFile::restored(original.path.clone(), original.baseline().cloned());
    fs::remove_file(&path).unwrap();
    assert!(matches!(
        file.try_save(&mine, &path, None).unwrap(),
        SaveOutcome::Conflict(DiskVersion::Missing)
    ));
    assert!(!path.exists() && file.dirty(&mine));
    Model::default().save(&path).unwrap();
    assert!(matches!(
        file.try_save(&mine, &path, Some(&DiskVersion::Missing))
            .unwrap(),
        SaveOutcome::Conflict(_)
    ));
    for bytes in [b"invalid JSON".to_vec(), vec![0; 4 * 1024 * 1024 + 1]] {
        fs::write(&path, &bytes).unwrap();
        assert!(file.try_save(&mine, &path, None).is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert!(file.dirty(&mine));
    }
    assert!(
        file.try_save(&mine, &dir.path().join("wrong.slx"), None)
            .is_err()
    );
    assert!(!dir.path().join("wrong.slx").exists());
    assert!(
        file.try_save(&mine, &dir.path().join("missing/copy.mrsim"), None)
            .is_err()
    );
    assert_eq!(file.path, original.path);
    let copy = dir.path().join("copy.mrsim");
    assert!(matches!(
        file.try_save(&mine, &copy, None).unwrap(),
        SaveOutcome::Saved
    ));
    assert!(!file.dirty(&mine));
    let mut legacy = ModelFile::restored(Some(copy.clone()), None);
    assert!(legacy.dirty(&mine));
    assert!(matches!(
        legacy.try_save(&mine, &copy, None).unwrap(),
        SaveOutcome::Conflict(_)
    ));
}
