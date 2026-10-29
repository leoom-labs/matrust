//! Release allocation+construction/SVG timing; no renderer or MATLAB comparison.
use matrust::{
    Matrix,
    plot::{Figure, Series, TextBox},
};
use std::{hint::black_box, sync::atomic::AtomicBool, time::Instant};
fn main() {
    if cfg!(debug_assertions) {
        panic!("Use --release");
    }
    let reverse = std::env::args().any(|a| a == "--reverse");
    let mut sizes = vec![0usize, 1, 64, 10000];
    if reverse {
        sizes.reverse();
    }
    let workers = if reverse { [8, 1] } else { [1, 8] };
    println!("workers,faces,operation,sample,elapsed_ns");
    for workers in workers {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        for &faces in &sizes {
            let x = Matrix::from_column_vec(
                4,
                faces,
                (0..faces)
                    .flat_map(|i| [i as f64, i as f64 + 0.5, i as f64 + 0.5, i as f64])
                    .collect(),
            )
            .unwrap();
            let y = Matrix::from_column_vec(
                4,
                faces,
                (0..faces)
                    .flat_map(|i| [0., 0., (i % 17) as f64 + 1., (i % 17) as f64 + 1.])
                    .collect(),
            )
            .unwrap();
            let cancel = AtomicBool::new(false);
            let mut figure = Figure::default();
            figure
                .add(vec![
                    Series::patch_from_matrices(&x, &y, [40, 80, 200], &cancel).unwrap(),
                ])
                .unwrap();
            figure
                .add_textbox(
                    TextBox::new([0., 0.94, 1., 0.06], "Actual face geometry".into()).unwrap(),
                )
                .unwrap();
            let reference = figure.svg();
            let ops = if reverse {
                ["svg", "construct"]
            } else {
                ["construct", "svg"]
            };
            for op in ops {
                for sample in 0..9 {
                    // Timer includes entering the pre-existing pool, allocation and
                    // the selected serial bounded operation; output drops after it.
                    let start = Instant::now();
                    let output = pool.install(|| {
                        if op == "construct" {
                            (
                                Some(
                                    Series::patch_from_matrices(
                                        black_box(&x),
                                        black_box(&y),
                                        [40, 80, 200],
                                        &cancel,
                                    )
                                    .unwrap(),
                                ),
                                None,
                            )
                        } else {
                            (None, Some(black_box(&figure).svg()))
                        }
                    });
                    let elapsed = start.elapsed().as_nanos();
                    if let Some(s) = &output.0 {
                        assert_eq!(s.patch_face_count(), faces);
                        assert!(
                            s.points
                                .iter()
                                .enumerate()
                                .all(|(i, p)| p[0].to_bits() == x.data()[i].to_bits()
                                    && p[1].to_bits() == y.data()[i].to_bits())
                        );
                    }
                    if let Some(svg) = &output.1 {
                        assert_eq!(svg, &reference);
                    }
                    if sample >= 2 {
                        println!("{workers},{faces},{op},{},{elapsed}", sample - 2);
                    }
                    drop(output);
                }
            }
        }
    }
}
