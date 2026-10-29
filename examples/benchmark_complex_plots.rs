//! Release-only coordinate construction comparison; no MATLAB baseline.
use matrust::{
    Complex64, ComplexMatrix,
    plot::{self, PlotData, PlotKind, Series},
};
use std::{hint::black_box, sync::atomic::AtomicBool, time::Instant};

fn main() {
    if cfg!(debug_assertions) {
        panic!("Use --release");
    }
    let reverse = std::env::args().any(|a| a == "--reverse");
    let mut workers = [1, 8];
    let mut shapes = [
        (1024, 1),
        (12499, 8),
        (12500, 8),
        (12501, 8),
        (1_000_000, 1),
        (125000, 8),
    ];
    if reverse {
        workers.reverse();
        shapes.reverse();
    }
    println!(
        "rows,cols,workers,mode,implementation,repetitions,median_ms,min_ms,max_ms,samples_ms"
    );
    for workers in workers {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        pool.install(|| {
            for (rows, cols) in shapes {
                let cancel = AtomicBool::new(false);
                let z = ComplexMatrix::from_column_vec(
                    rows,
                    cols,
                    (0..rows * cols)
                        .map(|i| Complex64::new((i % 1009) as f64, -((i % 1013) as f64)))
                        .collect(),
                    &cancel,
                )
                .unwrap();
                let pointer = z.data().as_ptr();
                for explicit in [false, true] {
                    let run = |direct: bool| -> Vec<Series> {
                        if direct {
                            plot::series_from_plot_data(
                                if explicit {
                                    Some(PlotData::Complex(&z))
                                } else {
                                    None
                                },
                                PlotData::Complex(&z),
                                &cancel,
                            )
                            .unwrap()
                        } else {
                            let x = z.real(&cancel).unwrap();
                            let y = if explicit {
                                z.real(&cancel).unwrap()
                            } else {
                                z.imag(&cancel).unwrap()
                            };
                            plot::series_from_matrices(Some(&x), &y, PlotKind::Line).unwrap()
                        }
                    };
                    let validate = |result: &[Series]| {
                        assert_eq!(result.len(), cols);
                        for (column, series) in result.iter().enumerate() {
                            assert_eq!(series.points.len(), rows);
                            for (row, point) in series.points.iter().enumerate() {
                                let value = z.data()[column * rows + row];
                                assert_eq!(point[0].to_bits(), value.re.to_bits());
                                assert_eq!(
                                    point[1].to_bits(),
                                    if explicit { value.re } else { value.im }.to_bits()
                                );
                            }
                        }
                    };
                    for _ in 0..2 {
                        for direct in [false, true] {
                            validate(&run(direct));
                        }
                    }
                    let mut samples = [Vec::new(), Vec::new()];
                    for repetition in 0..7 {
                        let order = if (repetition % 2 == 0) ^ reverse {
                            [false, true]
                        } else {
                            [true, false]
                        };
                        for direct in order {
                            let start = Instant::now();
                            let result = black_box(run(direct));
                            let elapsed = start.elapsed().as_secs_f64() * 1000.;
                            validate(&result);
                            drop(result);
                            samples[usize::from(direct)].push(elapsed);
                        }
                    }
                    assert_eq!(z.data().as_ptr(), pointer);
                    for (direct, values) in samples.iter().enumerate() {
                        let mut sorted = values.clone();
                        sorted.sort_by(f64::total_cmp);
                        println!(
                            "{rows},{cols},{workers},{},{},7,{:.9},{:.9},{:.9},{}",
                            if explicit {
                                "explicit_real_parts"
                            } else {
                                "complex_plane"
                            },
                            if direct == 1 {
                                "borrowed_direct"
                            } else {
                                "component_extraction"
                            },
                            sorted[3],
                            sorted[0],
                            sorted[6],
                            values
                                .iter()
                                .map(|x| format!("{x:.9}"))
                                .collect::<Vec<_>>()
                                .join(";")
                        );
                    }
                }
            }
        });
    }
}
