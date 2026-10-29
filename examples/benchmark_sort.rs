//! Candidate versus one standard stable sort per lane. Research threshold is provisional.
use matrust::{
    Array, Matrix,
    array::{MissingPlacement, SortOrder},
};
use std::{hint::black_box, sync::atomic::AtomicBool, time::Instant};

fn direct(
    a: &Array,
    dim: usize,
    descending: bool,
    want_indices: bool,
) -> (Vec<f64>, Option<Vec<f64>>) {
    let (r, c) = (a.rows(), a.cols());
    let mut output = vec![0.; a.len()];
    let mut indices = want_indices.then(|| vec![0.; a.len()]);
    let (lanes, n) = if dim == 1 { (c, r) } else { (r, c) };
    for lane in 0..lanes {
        let mut pairs: Vec<_> = (0..n)
            .map(|at| {
                (
                    if dim == 1 {
                        a.scalar(at, lane).unwrap().into_real().unwrap()
                    } else {
                        a.scalar(lane, at).unwrap().into_real().unwrap()
                    },
                    at,
                )
            })
            .collect();
        pairs.sort_by(|a, b| {
            if descending {
                b.0.partial_cmp(&a.0).unwrap()
            } else {
                a.0.partial_cmp(&b.0).unwrap()
            }
        });
        for (at, (value, original)) in pairs.into_iter().enumerate() {
            let out = if dim == 1 {
                lane * r + at
            } else {
                lane + at * r
            };
            output[out] = value;
            if let Some(indices) = &mut indices {
                indices[out] = original as f64 + 1.;
            }
        }
    }
    (output, indices)
}

fn main() {
    let reverse = std::env::args().skip(1).any(|a| a == "--reverse");
    let cancel = AtomicBool::new(false);
    let pools = [1, 8].map(|n| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build()
            .unwrap()
    });
    let mut cases = vec![
        (16384, 1, 1),
        (65536, 1, 1),
        (524287, 1, 1),
        (524289, 1, 1),
        (8193, 64, 1),
        (64, 8193, 2),
        (128, 4096, 1),
    ];
    if reverse {
        cases.reverse();
    }
    println!(
        "implementation,rows,cols,dim,workers,pattern,descending,indices,block,position,nanoseconds"
    );
    for (rows, cols, dim) in cases {
        for pattern in ["repeated", "ordered", "reversed"] {
            let a: Array = Matrix::from_column_vec(
                rows,
                cols,
                (0..rows * cols)
                    .map(|i| match pattern {
                        "ordered" => i as f64,
                        "reversed" => (rows * cols - i) as f64,
                        _ => ((i * 7919) % 997) as f64,
                    })
                    .collect(),
            )
            .unwrap()
            .into();
            for descending in [false, true] {
                for want_indices in [false, true] {
                    let expected = direct(&a, dim, descending, want_indices);
                    for (p, pool) in pools.iter().enumerate() {
                        let workers = [1, 8][p];
                        let run = || {
                            a.sort_dimension(
                                dim,
                                if descending {
                                    SortOrder::Descend
                                } else {
                                    SortOrder::Ascend
                                },
                                MissingPlacement::Auto,
                                want_indices,
                                &cancel,
                            )
                            .unwrap()
                        };
                        let actual = pool.install(run);
                        assert_eq!(actual.0.as_numeric().unwrap().data(), expected.0);
                        assert_eq!(actual.1.as_ref().map(|m| m.data()), expected.1.as_deref());
                        for _ in 0..2 {
                            black_box(pool.install(run));
                            black_box(direct(&a, dim, descending, want_indices));
                        }
                        for block in 0..7 {
                            let sequence = if reverse { [1, 0, 0, 1] } else { [0, 1, 1, 0] };
                            for (position, implementation) in sequence.into_iter().enumerate() {
                                let start = Instant::now();
                                if implementation == 0 {
                                    black_box(pool.install(run));
                                } else {
                                    black_box(direct(&a, dim, descending, want_indices));
                                }
                                let ns = start.elapsed().as_nanos();
                                println!(
                                    "{},{rows},{cols},{dim},{workers},{pattern},{descending},{want_indices},{block},{position},{ns}",
                                    if implementation == 0 {
                                        "candidate"
                                    } else {
                                        "direct_stable"
                                    }
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}
