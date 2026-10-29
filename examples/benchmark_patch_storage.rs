//! Same geometry, dense pair storage versus verified coordinate storage.
//! Setup, correctness, source identity and output destruction are outside timers.
use matrust::{
    Matrix,
    plot::{AxisTransform, Figure, Series},
};
use std::{hint::black_box, sync::atomic::AtomicBool, time::Instant};

fn inputs(count: usize) -> (Matrix, Matrix) {
    let x =
        Matrix::from_column_vec(count, 1, (0..count).map(|i| i as f64 + 0.25).collect()).unwrap();
    let y = Matrix::from_column_vec(
        count,
        1,
        (0..count)
            .map(|i| if i == 0 { -0.0 } else { (i % 31) as f64 - 7. })
            .collect(),
    )
    .unwrap();
    let left = x.coordinate_subtract(&Matrix::scalar(0.4)).unwrap();
    let right = x.coordinate_add(&Matrix::scalar(0.4)).unwrap();
    let base = y.coordinate_multiply(&Matrix::scalar(0.)).unwrap();
    (
        Matrix::concatenate(vec![vec![left.clone(), right.clone(), right, left]])
            .unwrap()
            .transpose(),
        Matrix::concatenate(vec![vec![base.clone(), base, y.clone(), y]])
            .unwrap()
            .transpose(),
    )
}
enum Output {
    Series(Series),
    Display(Vec<[f64; 2]>),
    Svg(String),
}
fn main() {
    if cfg!(debug_assertions) {
        panic!("Use a release artifact");
    }
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    assert!(args.is_empty() || args == ["--reverse"]);
    let mut schedule = Vec::new();
    for faces in [1, 256, 10_000] {
        for workers in [1, 8] {
            for storage in ["compact", "dense_control"] {
                for operation in ["construct", "display", "svg"] {
                    schedule.push((faces, workers, storage, operation));
                }
            }
        }
    }
    if !args.is_empty() {
        schedule.reverse();
    }
    let stop = AtomicBool::new(false);
    let axes = [AxisTransform::default(); 2];
    println!(
        "faces\tworkers\tstorage\toperation\tsample\tnanoseconds\tsource_units\tretained_payload_bytes\toutput_bytes"
    );
    for (faces, workers, storage, operation) in schedule {
        let (proof_x, proof_y) = inputs(faces);
        let x = if storage == "compact" {
            proof_x.clone()
        } else {
            Matrix::from_column_slice(4, faces, proof_x.data()).unwrap()
        };
        let y = if storage == "compact" {
            proof_y.clone()
        } else {
            Matrix::from_column_slice(4, faces, proof_y.data()).unwrap()
        };
        let alias_x = x.clone();
        let alias_y = y.clone();
        let series = Series::patch_from_matrices(&x, &y, [31, 73, 127], &stop).unwrap();
        assert_eq!(series.points.patch().is_some(), storage == "compact");
        let mut figure = Figure::default();
        figure.add(vec![series.clone()]).unwrap();
        let mut dense = figure.clone();
        dense.series[0].points = series.points.materialize(&stop).unwrap().into();
        let expected_svg = dense.svg();
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        for repetition in 0..9 {
            let start = Instant::now();
            let output = pool.install(|| match operation {
                "construct" => Output::Series(
                    Series::patch_from_matrices(&x, &y, [31, 73, 127], &stop).unwrap(),
                ),
                "display" => Output::Display(
                    series
                        .display_points_axes_cancellable(8192, axes, &stop)
                        .unwrap(),
                ),
                "svg" => Output::Svg(figure.svg()),
                _ => unreachable!(),
            });
            let elapsed = start.elapsed().as_nanos();
            black_box(&output);
            let output_bytes = match &output {
                Output::Series(actual) => {
                    assert_eq!(actual.points.len(), faces * 4);
                    assert_eq!(actual.points.patch().is_some(), storage == "compact");
                    for (index, point) in actual.points.iter().enumerate() {
                        assert_eq!(
                            point.map(f64::to_bits),
                            [
                                proof_x.data()[index].to_bits(),
                                proof_y.data()[index].to_bits()
                            ]
                        );
                    }
                    let mut bytes = 0;
                    actual.points.visit_retained_allocations(|_, n| bytes += n);
                    bytes
                }
                Output::Display(actual) => {
                    assert_eq!(actual.len(), faces * 4);
                    for (index, point) in actual.iter().enumerate() {
                        assert_eq!(
                            point.map(f64::to_bits),
                            [
                                proof_x.data()[index].to_bits(),
                                proof_y.data()[index].to_bits()
                            ]
                        );
                    }
                    actual.capacity() * 16
                }
                Output::Svg(actual) => {
                    assert_eq!(actual, &expected_svg);
                    actual.len()
                }
            };
            assert_eq!(x.data().as_ptr(), alias_x.data().as_ptr());
            assert_eq!(y.data().as_ptr(), alias_y.data().as_ptr());
            let cost = figure.geometry_ledger().unwrap().cost();
            if repetition >= 2 {
                println!(
                    "{faces}\t{workers}\t{storage}\t{operation}\t{}\t{elapsed}\t{}\t{}\t{output_bytes}",
                    repetition - 2,
                    cost.source_units(),
                    cost.retained_bytes()
                );
            }
            drop(output);
        }
    }
}
