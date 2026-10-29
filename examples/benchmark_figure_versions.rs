use matrust::{
    Matrix,
    files::{
        self,
        figure_version::{self as tracked, DiskVersion},
    },
    plot::Figure,
};
use std::{hint::black_box, sync::atomic::AtomicBool, time::Instant};
fn median(times: &mut [f64]) -> f64 {
    times.sort_by(f64::total_cmp);
    times[times.len() / 2]
}
fn main() {
    let dir = tempfile::tempdir().unwrap();
    let cancel = AtomicBool::new(false);
    println!("operation,format,points,file_bytes,repetitions,median_ms");
    for points in [1000, 100_000, 1_000_000] {
        let x = Matrix::from_column_slice(
            points,
            1,
            &(0..points).map(|i| i as f64).collect::<Vec<_>>(),
        )
        .unwrap();
        let mut figure = Figure::new(&x, &x).unwrap();
        figure.title = "Versioned figure".into();
        for extension in ["mfig", "fig"] {
            let path = dir.path().join(format!("plot.{extension}"));
            files::save_figure(&path, &figure).unwrap();
            let expected = tracked::inspect(&path, &cancel).unwrap();
            let DiskVersion::Present(v) = expected else {
                panic!()
            };
            for operation in ["load", "save", "check"] {
                let repetitions = if operation == "save" { 11 } else { 21 };
                let mut old = Vec::new();
                let mut new = Vec::new();
                for i in 0..repetitions + 3 {
                    let order = if i % 2 == 0 {
                        [false, true]
                    } else {
                        [true, false]
                    };
                    for protected in order {
                        if operation == "check" && !protected {
                            continue;
                        }
                        let start = Instant::now();
                        match (operation, protected) {
                            ("load", false) => {
                                black_box(files::load_figure(&path).unwrap());
                            }
                            ("load", true) => {
                                black_box(tracked::load(&path, &cancel).unwrap());
                            }
                            ("save", false) => files::save_figure(&path, &figure).unwrap(),
                            ("save", true) => {
                                assert!(matches!(
                                    tracked::save(&path, &figure, Some(expected), &cancel).unwrap(),
                                    tracked::SaveOutcome::Saved(_)
                                ));
                            }
                            _ => assert_eq!(
                                black_box(tracked::inspect(&path, &cancel).unwrap()),
                                expected
                            ),
                        }
                        let elapsed = start.elapsed().as_secs_f64() * 1000.;
                        if i >= 3 {
                            if protected {
                                new.push(elapsed);
                            } else {
                                old.push(elapsed);
                            }
                        }
                    }
                }
                if !old.is_empty() {
                    println!(
                        "{operation}_untracked,{extension},{points},{},{repetitions},{:.6}",
                        v.bytes,
                        median(&mut old)
                    );
                }
                println!(
                    "{operation}_tracked,{extension},{points},{},{repetitions},{:.6}",
                    v.bytes,
                    median(&mut new)
                );
            }
            assert_eq!(
                tracked::load(&path, &cancel).unwrap().0.series[0].points,
                figure.series[0].points
            );
        }
    }
}
