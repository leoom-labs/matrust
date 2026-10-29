//! Whole public construction/reconstruction costs, not an isolated kernel claim.
use matrust::{
    Engine,
    document::Document,
    files,
    plot::{Figure, PlotKind},
};
use std::{hint::black_box, sync::atomic::AtomicBool, time::Instant};

fn verify(source: &Figure, actual: &Figure) {
    assert_eq!(source.series.len(), actual.series.len());
    for (a, b) in source.series.iter().zip(&actual.series) {
        assert_eq!(a.kind, PlotKind::Bar);
        assert_eq!(b.kind, PlotKind::Bar);
        assert_eq!(a.bar_width.to_bits(), b.bar_width.to_bits());
        assert_eq!(a.bar_offset.to_bits(), b.bar_offset.to_bits());
        for (x, y) in a.points.iter().zip(b.points.iter()) {
            assert_eq!(x.map(f64::to_bits), y.map(f64::to_bits));
        }
        assert_eq!(a.points.len(), b.points.len());
    }
}
fn main() {
    if cfg!(debug_assertions) {
        panic!("Use a release artifact");
    }
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    assert!(args.is_empty() || args == ["--reverse"]);
    let mut schedule = Vec::new();
    for bars in [4, 256, 10_000] {
        for workers in [1, 8] {
            for operation in [
                "legacy_construct_control",
                "relative_construct",
                "inline_encode",
                "bundle_publish",
            ] {
                schedule.push((bars, workers, operation));
            }
        }
    }
    if !args.is_empty() {
        schedule.reverse();
    }
    eprintln!(
        "hardware={};arch={};os={};logical_processors={};retained=5;warmups=2;reverse={}",
        std::env::var("PROCESSOR_IDENTIFIER").unwrap_or_else(|_| "unreported".into()),
        std::env::consts::ARCH,
        std::env::consts::OS,
        std::thread::available_parallelism().map_or(0, usize::from),
        !args.is_empty()
    );
    println!("bars\tworkers\toperation\tsample\tnanoseconds\tscript_bytes");
    let root = tempfile::tempdir().unwrap();
    let cwd = std::env::current_dir().unwrap();
    let cancel = AtomicBool::new(false);
    for (bars, workers, operation) in schedule {
        let rows = bars / 2;
        let prefix = format!("bar(1:{rows},[ones({rows},1) 2*ones({rows},1)]");
        let legacy = format!("{prefix});");
        let relative = format!("{prefix},'GroupWidth',.8,'BarWidth',1);");
        let mut engine = Engine::default();
        engine.execute(&legacy).unwrap();
        let source = engine.figure.unwrap();
        let snapshot = source.clone();
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        for repetition in 0..7 {
            let directory = tempfile::Builder::new()
                .prefix("bar_replay_")
                .tempdir_in(root.path())
                .unwrap();
            let path = directory.path().join("scene.m");
            let start = Instant::now();
            let (constructed, text, bundle) = pool.install(|| match operation {
                "legacy_construct_control" | "relative_construct" => {
                    let mut engine = Engine::default();
                    engine
                        .execute(if operation == "relative_construct" {
                            &relative
                        } else {
                            &legacy
                        })
                        .unwrap();
                    (Some(engine), None, None)
                }
                "inline_encode" => (
                    None,
                    Some(files::matlab_plot_script(&source, &cancel).unwrap()),
                    None,
                ),
                "bundle_publish" => (
                    None,
                    None,
                    Some(
                        files::write_matlab_plot_bundle_with_cancel(&path, &source, &cancel)
                            .unwrap(),
                    ),
                ),
                _ => unreachable!(),
            });
            let nanos = start.elapsed().as_nanos();
            black_box((&constructed, &text, &bundle));
            let bytes = if let Some(engine) = &constructed {
                verify(&source, engine.figure.as_ref().unwrap());
                0
            } else if let Some(text) = &text {
                let mut engine = Engine::default();
                engine.execute(text).unwrap();
                verify(&source, engine.figure.as_ref().unwrap());
                text.len()
            } else {
                let document = Document::open(&path).unwrap();
                let mut engine = Engine::default();
                engine.execute_document(&document.text, &path).unwrap();
                verify(&source, engine.figure.as_ref().unwrap());
                document.text.len()
            };
            assert!(source.same_snapshot(&snapshot));
            for (a, b) in source.series.iter().zip(&snapshot.series) {
                assert!(a.points.same_storage(&b.points));
            }
            assert_eq!(std::env::current_dir().unwrap(), cwd);
            if repetition >= 2 {
                println!(
                    "{bars}\t{workers}\t{operation}\t{}\t{nanos}\t{bytes}",
                    repetition - 2
                );
            }
            drop(constructed);
            drop(text);
            drop(bundle);
            directory.close().unwrap();
        }
    }
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    root.close().unwrap();
}
