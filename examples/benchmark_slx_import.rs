//! Opt-in release import timing: serial parser in observed 1/8 worker contexts.
use matrust::simulation::slx;
use std::{sync::atomic::AtomicBool, time::Instant};
fn main() {
    if cfg!(debug_assertions) {
        panic!("release profile required");
    }
    let args: Vec<_> = std::env::args().skip(1).collect();
    assert!(
        args.is_empty() || args == ["--reverse"],
        "only --reverse is supported"
    );
    let reverse = !args.is_empty();
    let dir = tempfile::tempdir().unwrap();
    let cases = [
        (
            "small",
            include_bytes!("../tests/fixtures/slx/slx-inherited-rate-admission-1526/chain.slx")
                .as_slice(),
            4usize,
        ),
        (
            "large",
            include_bytes!("../tests/fixtures/slx/slx-import-1512/large.slx").as_slice(),
            512usize,
        ),
    ];
    let mut tasks = Vec::new();
    for (name, bytes, nodes) in cases {
        let path = dir.path().join(format!("{name}.slx"));
        std::fs::write(&path, bytes).unwrap();
        let model = slx::import(&path, &AtomicBool::new(false)).unwrap().model;
        assert_eq!(model.nodes.len(), nodes);
        assert_eq!(model.steps, 5);
        let run = model.run(&AtomicBool::new(false)).unwrap();
        assert_eq!(
            run.traces[0].values.data(),
            if nodes == 4 {
                &[5., -6., -6., -6., -6.]
            } else {
                &[2., 2., 2., 2., 2.]
            }
        );
        for workers in [1, 8] {
            tasks.push((
                name,
                path.clone(),
                nodes,
                bytes.len(),
                workers,
                model.clone(),
            ));
        }
    }
    if reverse {
        tasks.reverse();
    }
    println!("order\tcase\tnodes\tarchive_bytes\tworkers\tphase\trepetition\tnanoseconds");
    for (name, path, nodes, bytes, workers, reference) in tasks {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        pool.install(|| {
            assert_eq!(rayon::current_num_threads(), workers);
            for repetition in 0..9 {
                let cancel = AtomicBool::new(false);
                let start = Instant::now();
                let imported = slx::import(&path, &cancel).unwrap();
                let ns = start.elapsed().as_nanos();
                assert!(imported.model.same_storage(&reference));
                println!(
                    "{}\t{name}\t{nodes}\t{bytes}\t{workers}\t{}\t{}\t{ns}",
                    if reverse { "reverse" } else { "normal" },
                    if repetition < 2 { "warmup" } else { "retained" },
                    if repetition < 2 {
                        repetition
                    } else {
                        repetition - 2
                    }
                );
                std::hint::black_box(imported);
            }
        });
    }
}
