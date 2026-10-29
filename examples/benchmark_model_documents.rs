use matrust::simulation::{
    Block, Model, Node,
    document::{ModelFile, SaveOutcome},
};
use std::{hint::black_box, time::Instant};
fn measure(mut f: impl FnMut(), repetitions: usize) -> f64 {
    for _ in 0..3 {
        f();
    }
    let mut times = Vec::new();
    for _ in 0..repetitions {
        let start = Instant::now();
        f();
        times.push(start.elapsed().as_secs_f64() * 1000.);
    }
    times.sort_by(f64::total_cmp);
    times[repetitions / 2]
}
fn main() {
    println!("operation,blocks,coefficients,file_bytes,repetitions,median_ms");
    let dir = tempfile::tempdir().unwrap();
    for large in [false, true] {
        let mut model = Model::default();
        if large {
            model.nodes = vec![Node {
                name: "Input".into(),
                block: Block::Constant { value: 1. },
                inputs: vec![],
            }];
            for i in 0..96 {
                model.nodes.push(Node {
                    name: format!("Filter {i}"),
                    block: Block::Filter {
                        b: vec![0.125; 2048],
                        a: vec![1.],
                    },
                    inputs: vec![0],
                });
            }
            model.nodes.push(Node {
                name: "Output".into(),
                block: Block::Scope,
                inputs: vec![96],
            });
        }
        let coefficients = model
            .nodes
            .iter()
            .map(|n| match &n.block {
                Block::Filter { b, a } => b.len() + a.len(),
                _ => 0,
            })
            .sum::<usize>();
        let path = dir.path().join("model.mrsim");
        model.save(&path).unwrap();
        let (_, mut file) = ModelFile::open(&path).unwrap();
        let original = serde_json::to_string(&model).unwrap();
        let bytes = std::fs::metadata(&path).unwrap().len();
        for operation in [
            "dirty_serialize_baseline",
            "dirty_compare",
            "check_disk",
            "save_checked",
        ] {
            let repetitions = if operation == "save_checked" { 15 } else { 31 };
            let time = measure(
                || match operation {
                    "dirty_serialize_baseline" => assert_eq!(
                        black_box(serde_json::to_string(black_box(&model)).unwrap()),
                        original
                    ),
                    "dirty_compare" => assert!(!black_box(file.dirty(black_box(&model)))),
                    "check_disk" => assert!(black_box(file.external_change().unwrap()).is_none()),
                    _ => assert!(matches!(
                        file.try_save(&model, &path, None).unwrap(),
                        SaveOutcome::Saved
                    )),
                },
                repetitions,
            );
            println!(
                "{operation},{},{coefficients},{bytes},{repetitions},{time:.6}",
                model.nodes.len()
            );
        }
        assert!(Model::load(&path).unwrap().same_storage(&model));
    }
}
