// Compile the same desktop history implementation without starting the GUI.
#[allow(dead_code)]
#[path = "../src/desktop/figure_history.rs"]
mod figure_history;
use figure_history::{History, METADATA_BUDGET, SAMPLE_BUDGET};
use matrust::Engine;
use std::{collections::BTreeSet, hint::black_box, time::Instant};
fn main() {
    let workers: usize = std::env::args()
        .nth(1)
        .unwrap_or("8".into())
        .parse()
        .unwrap();
    assert!([1, 8].contains(&workers));
    println!(
        "case,workers,samples,repetitions,median_ms,min_ms,max_ms,retained_sample_bytes,retained_metadata_bytes"
    );
    rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .unwrap()
        .install(|| {
            for count in [1000, 100000] {
                let mut e = Engine::default();
                e.execute(&format!(
                    "x=1:{count};tiledlayout(4,2);for k=1:8;nexttile;plot(x);end;"
                ))
                .unwrap();
                let source = e.figure.unwrap();
                let mut times = Vec::new();
                let mut retained = (0, 0);
                for repetition in 0..14 {
                    let mut history = History::new(true);
                    let mut current = source.clone();
                    let start = Instant::now();
                    for k in 0..1000 {
                        history.record(&current, k * 3);
                        current.title = format!("Edit {k}");
                        let undo = history.navigate(&current, false, k * 3 + 1).unwrap();
                        current = history.navigate(&undo, true, k * 3 + 2).unwrap();
                        let mut seen = BTreeSet::new();
                        let (mut samples, mut metadata) = (0, 0);
                        history.account(&mut seen, &mut samples, &mut metadata);
                        black_box((samples, metadata));
                        retained = (samples, metadata);
                    }
                    if repetition >= 3 {
                        times.push(start.elapsed().as_secs_f64() * 1000.);
                    }
                    assert_eq!(history.counts(), (32, 0));
                    assert!(retained.0 <= SAMPLE_BUDGET && retained.1 <= METADATA_BUDGET);
                    assert_eq!(current.title, "Edit 999");
                    assert_eq!(current.point_count(), count * 8);
                }
                times.sort_by(f64::total_cmp);
                println!(
                    "1000_edit_undo_redo_account,{workers},{},11,{:.6},{:.6},{:.6},{},{}",
                    count * 8,
                    times[5],
                    times[0],
                    times[10],
                    retained.0,
                    retained.1
                );
            }
        });
}
