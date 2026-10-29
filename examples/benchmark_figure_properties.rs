//! Bounded release measurements; ordinary legacy comparison is a separate probe.
use matrust::plot::{Figure, PlotKind, Series};
use std::{hint::black_box, time::Instant};
fn main() {
    if cfg!(debug_assertions) {
        panic!("Release build required");
    }
    let reverse = std::env::args().any(|a| a == "--reverse");
    println!("workers,points,base,color,operation,sample,total_elapsed_ns");
    for workers in if reverse { [8, 1] } else { [1, 8] } {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        for count in if reverse {
            [65536, 1024]
        } else {
            [1024, 65536]
        } {
            for explicit in if reverse {
                [true, false]
            } else {
                [false, true]
            } {
                for colored in if reverse {
                    [true, false]
                } else {
                    [false, true]
                } {
                    let mut figure = Figure {
                        name: "Property benchmark".into(),
                        ..Default::default()
                    };
                    if colored {
                        figure.set_canvas_color([80, 120, 160]);
                    }
                    let mut series = Series::new(
                        (0..count)
                            .map(|i| [i as f64, (i % 257) as f64 - 128.])
                            .collect(),
                        PlotKind::Stem,
                    );
                    if explicit {
                        series.base_value = Some(-200.);
                    }
                    figure.add(vec![series]).unwrap();
                    figure.validate().unwrap();
                    let expected = figure.svg();
                    let bounds = figure.bounds();
                    for operation in if reverse {
                        ["svg", "bounds"]
                    } else {
                        ["bounds", "svg"]
                    } {
                        for sample in 0..9 {
                            let start = Instant::now();
                            let result = pool.install(|| {
                                if operation == "svg" {
                                    (Some(black_box(&figure).svg()), None)
                                } else {
                                    (None, Some(black_box(&figure).bounds()))
                                }
                            });
                            let elapsed = start.elapsed().as_nanos();
                            if let Some(svg) = &result.0 {
                                assert_eq!(svg, &expected);
                            }
                            if let Some(actual) = result.1 {
                                assert_eq!(actual, bounds);
                            }
                            if sample >= 2 {
                                println!(
                                    "{workers},{count},{},{},{operation},{},{elapsed}",
                                    if explicit { "explicit" } else { "automatic" },
                                    if colored { "rgb" } else { "white" },
                                    sample - 2
                                );
                            }
                            drop(result); // outside timer, along with setup and correctness checks
                        }
                    }
                }
            }
        }
    }
}
