use matrust::document::{Document, SaveOutcome};
use std::{hint::black_box, time::Instant};
fn measure(mut f: impl FnMut()) -> (f64, f64, f64) {
    for _ in 0..2 {
        f();
    }
    let mut times = Vec::new();
    for _ in 0..15 {
        let start = Instant::now();
        f();
        times.push(start.elapsed().as_secs_f64() * 1000.);
    }
    times.sort_by(f64::total_cmp);
    (times[7], times[0], times[14])
}
fn main() {
    println!("operation,bytes,repetitions,median_ms,min_ms,max_ms");
    let dir = tempfile::tempdir().unwrap();
    for bytes in [1024, 1_000_000] {
        let path = dir.path().join("source.m");
        let source = format!("%{}", "x".repeat(bytes - 1));
        std::fs::write(&path, &source).unwrap();
        let mut document = Document::open(&path).unwrap();
        for operation in ["check_unchanged", "check_changed", "save_with_check"] {
            if operation == "check_changed" {
                std::fs::write(&path, source.replace('x', "y")).unwrap();
            }
            if operation == "save_with_check" {
                std::fs::write(&path, &source).unwrap();
            }
            let (median, min, max) = measure(|| {
                if operation == "save_with_check" {
                    assert_eq!(document.try_save(&path, None).unwrap(), SaveOutcome::Saved);
                } else {
                    let change = black_box(document.external_change().unwrap());
                    assert_eq!(change.is_some(), operation == "check_changed");
                }
            });
            println!("{operation},{bytes},15,{median:.6},{min:.6},{max:.6}");
        }
        assert_eq!(std::fs::read_to_string(path).unwrap(), source);
    }
}
