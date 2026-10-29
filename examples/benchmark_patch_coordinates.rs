//! Source-only staged foundation harness; run only a parent-accepted release build.
use matrust::plot::coordinates::{Coordinate, CoordinateLane, PatchCoordinates};
use std::{
    hint::black_box,
    sync::{Arc, atomic::AtomicBool},
    time::Instant,
};

fn corners(points: &Arc<Vec<[f64; 2]>>) -> [[Coordinate; 2]; 4] {
    use matrust::plot::coordinates::ScalarStep;
    let x = CoordinateLane::pair_component(points, 0).unwrap();
    let y = CoordinateLane::pair_component(points, 1).unwrap();
    let left = x
        .clone()
        .then(ScalarStep::Add(0.25))
        .unwrap()
        .then(ScalarStep::Subtract(0.4))
        .unwrap();
    let right = x
        .then(ScalarStep::Add(0.25))
        .unwrap()
        .then(ScalarStep::Add(0.4))
        .unwrap();
    [
        [Coordinate::Lane(left.clone()), Coordinate::Constant(-0.0)],
        [Coordinate::Lane(right.clone()), Coordinate::Constant(-0.0)],
        [Coordinate::Lane(right), Coordinate::Lane(y.clone())],
        [Coordinate::Lane(left), Coordinate::Lane(y)],
    ]
}
enum Output {
    Geometry(PatchCoordinates),
    Points(Vec<[f64; 2]>),
}
fn main() {
    if cfg!(debug_assertions) {
        panic!("Use a release artifact");
    }
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    assert!(args.is_empty() || args == ["--reverse"]);
    let stop = AtomicBool::new(false);
    let mut schedule = Vec::new();
    for faces in [1, 256, 10_000] {
        for workers in [1, 8] {
            for operation in ["dense_validate", "view_validate", "view_materialize"] {
                schedule.push((faces, workers, operation));
            }
        }
    }
    if !args.is_empty() {
        schedule.reverse();
    }
    println!(
        "faces\tworkers\toperation\tsample\tnanoseconds\tsource_bytes\tdense_bytes\tdescriptor_bytes"
    );
    for (faces, workers, operation) in schedule {
        let points: Arc<Vec<[f64; 2]>> = Arc::new(
            (0..faces)
                .map(|i| {
                    [
                        i as f64 + 0.125,
                        if i == 0 { -0.0 } else { (i % 19) as f64 - 7.0 },
                    ]
                })
                .collect(),
        );
        let alias = points.clone();
        let descriptor = corners(&points);
        let view = PatchCoordinates::rectangles(descriptor.clone(), faces, &stop).unwrap();
        // Independent eager arithmetic reference, outside timing.
        let expected = Arc::new(
            points
                .iter()
                .flat_map(|[x, y]| {
                    [
                        [(x + 0.25) - 0.4, -0.0],
                        [(x + 0.25) + 0.4, -0.0],
                        [(x + 0.25) + 0.4, *y],
                        [(x + 0.25) - 0.4, *y],
                    ]
                })
                .collect::<Vec<_>>(),
        );
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        for repetition in 0..9 {
            let started = Instant::now();
            let output = pool.install(|| match operation {
                "dense_validate" => {
                    Output::Geometry(PatchCoordinates::dense(expected.clone(), 4, &stop).unwrap())
                }
                "view_validate" => Output::Geometry(
                    PatchCoordinates::rectangles(descriptor.clone(), faces, &stop).unwrap(),
                ),
                "view_materialize" => Output::Points(view.materialize(&stop).unwrap()),
                _ => unreachable!(),
            });
            let elapsed = started.elapsed().as_nanos();
            black_box(&output);
            let actual = match &output {
                Output::Geometry(geometry) => geometry.materialize(&stop).unwrap(),
                Output::Points(values) => values.clone(),
            };
            assert_eq!(actual.len(), expected.len());
            for (a, b) in actual.iter().zip(expected.iter()) {
                assert_eq!(a.map(f64::to_bits), b.map(f64::to_bits));
            }
            assert!(Arc::ptr_eq(&points, &alias));
            assert_eq!(points.len(), faces);
            if repetition >= 2 {
                println!(
                    "{faces}\t{workers}\t{operation}\t{}\t{elapsed}\t{}\t{}\t{}",
                    repetition - 2,
                    points.capacity() * 16,
                    expected.capacity() * 16,
                    view.descriptor_bytes()
                );
            }
            drop(actual);
            drop(output);
        }
    }
}
