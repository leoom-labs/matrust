//! Release, end-to-end assignment timings; copies/snapshots are prepared outside timing.
use matrust::{Engine, Matrix};
use std::{hint::black_box, time::Instant};

fn main() {
    println!("case,elements,threads,repetitions,median_ms,min_ms,max_ms");
    for threads in [1, 8] {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(|| {
                for len in [65536, 1048576, 8388608] {
                    let rows = len / 8;
                    for (name, script) in [
                        ("unique_cell", "A(2,3)=7;"),
                        ("shared_cell", "A(2,3)=7;"),
                        ("short_slice", "A(1:4096,3)=7;"),
                        ("column", "A(:,3)=7;"),
                        ("fill", "A(:)=7;"),
                        ("reverse_rhs", "A(end:-1:1)=R;"),
                        ("growth", "A(:,9)=7;"),
                        ("delete_column", "A(:,4)=[];"),
                    ] {
                        let mut e = Engine::default();
                        e.variables
                            .insert("A".into(), Matrix::filled(rows, 8, 0.).unwrap());
                        if name == "reverse_rhs" {
                            e.variables.insert(
                                "R".into(),
                                Matrix::from_column_vec(
                                    1,
                                    len,
                                    (0..len).map(|i| i as f64).collect(),
                                )
                                .unwrap(),
                            );
                        }
                        let reset = matches!(name, "growth" | "delete_column")
                            .then(|| e.variables["A"].clone());
                        let prepare = |e: &mut Engine| {
                            if let Some(reset) = &reset {
                                e.variables.insert("A".into(), reset.clone());
                            }
                            if name == "shared_cell" {
                                e.variables
                                    .insert("snapshot".into(), e.variables["A"].clone());
                            }
                        };
                        prepare(&mut e);
                        e.execute(script).unwrap();
                        let a = &e.variables["A"];
                        let cols = match name {
                            "growth" => 9,
                            "delete_column" => 7,
                            _ => 8,
                        };
                        assert_eq!((a.rows(), a.cols()), (rows, cols));
                        for (i, &x) in a.data().iter().enumerate() {
                            let expected = match name {
                                "unique_cell" | "shared_cell" if i == 2 * rows + 1 => 7.,
                                "short_slice" if i / rows == 2 && i % rows < 4096 => 7.,
                                "column" if i / rows == 2 => 7.,
                                "fill" => 7.,
                                "reverse_rhs" => (len - i - 1) as f64,
                                "growth" if i / rows == 8 => 7.,
                                _ => 0.,
                            };
                            assert_eq!(x, expected, "{name} at {i}");
                        }
                        let batch = if matches!(name, "unique_cell" | "short_slice") {
                            100
                        } else {
                            1
                        };
                        let mut times = Vec::new();
                        for rep in 0..33 {
                            let mut elapsed = 0.;
                            for _ in 0..batch {
                                prepare(&mut e);
                                let start = Instant::now();
                                black_box(e.execute(black_box(script)).unwrap());
                                elapsed += start.elapsed().as_secs_f64() * 1000.;
                            }
                            if rep >= 3 {
                                times.push(elapsed / batch as f64);
                            }
                        }
                        times.sort_by(f64::total_cmp);
                        println!(
                            "{name},{len},{threads},30,{:.6},{:.6},{:.6}",
                            (times[14] + times[15]) * 0.5,
                            times[0],
                            times[29]
                        );
                    }
                }
            });
    }
}
