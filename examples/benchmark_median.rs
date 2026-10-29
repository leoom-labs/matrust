//! Parent-scheduled release median experiment. No MATLAB timing claim.
//! Candidate vs serial full-lane standard selection is a memory/scheduling
//! tradeoff comparison; the control has no cooperative cancellation.
use matrust::{Matrix, matrix::MedianAxis as Axis};
use std::{sync::atomic::AtomicBool, time::Instant};

fn control(input: &Matrix, axis: Axis) -> Matrix {
    let (rows, cols, length, stride) = match axis {
        Axis::All => (1, 1, input.len(), 1),
        Axis::Dimension(1) => (1, input.cols(), input.rows(), 1),
        Axis::Dimension(2) => (input.rows(), 1, input.cols(), input.rows()),
        _ => unreachable!(),
    };
    let mut output = Vec::with_capacity(rows * cols);
    let mut scratch = Vec::with_capacity(length);
    for lane in 0..rows * cols {
        scratch.clear();
        let start = if stride == 1 { lane * length } else { lane };
        scratch.extend(
            (0..length)
                .map(|i| input.data()[start + i * stride])
                .filter(|v| !v.is_nan()),
        );
        let n = scratch.len();
        let result = if n == 0 {
            f64::NAN
        } else {
            let (left, middle, _) = scratch.select_nth_unstable_by(n / 2, f64::total_cmp);
            if n.is_multiple_of(2) {
                left.iter()
                    .copied()
                    .max_by(f64::total_cmp)
                    .unwrap()
                    .midpoint(*middle)
            } else {
                *middle
            }
        };
        output.push(result);
    }
    Matrix::from_column_vec(rows, cols, output).unwrap()
}

fn main() {
    if cfg!(debug_assertions) {
        panic!("Use a release build");
    }
    let reverse = std::env::args().any(|a| a == "--reverse");
    let mut workers = [1, 8];
    if reverse {
        workers.reverse();
    }
    println!(
        "rows,cols,axis,pattern,workers,implementation,repetitions,median_us,min_us,max_us,samples_us"
    );
    for workers in workers {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        pool.install(|| {
            let mut shapes=[(64,8,1),(8192,8,1),(8193,8,1),(8192,64,1),(8193,64,1),(64,8193,2),(65536,1,0),(262144,1,0)];
            if reverse { shapes.reverse(); }
            for (rows,cols,axis_number) in shapes {
                let axis=match axis_number {0=>Axis::All,1=>Axis::Dimension(1),2=>Axis::Dimension(2),_=>unreachable!()};
                for pattern in ["ordinary","duplicates","missing_extreme"] {
                    let input=Matrix::from_column_vec(rows,cols,(0..rows*cols).map(|i| match pattern {
                        "ordinary" => ((i*173)%65521) as f64 / 16. - 2048.,
                        "duplicates" => (i%3) as f64,
                        _ => match i%23 { 0=>f64::NAN,1=>f64::MAX,2=>-f64::MAX,3=>-0.,4=>0.,_=>((i*17)%127) as f64 },
                    }).collect()).unwrap();
                    let expected=control(&input,axis); let cancel=AtomicBool::new(false);
                    let mut samples=[Vec::new(),Vec::new()];
                    for round in 0..9 {
                        let order=if (round%2==0)^reverse {[0,1,1,0]} else {[1,0,0,1]};
                        for implementation in order {
                            let start=Instant::now();
                            let output=if implementation==0 {input.median(axis,true,&cancel).unwrap()} else {control(&input,axis)};
                            let elapsed=start.elapsed().as_secs_f64()*1e6;
                            if round>=2 { samples[implementation].push(elapsed); }
                            assert_eq!((output.rows(),output.cols()),(expected.rows(),expected.cols()));
                            for (&a,&b) in output.data().iter().zip(expected.data()) { assert!(a.to_bits()==b.to_bits() || a.is_nan()&&b.is_nan()); }
                            drop(output);
                        }
                    }
                    for (name,samples) in ["candidate","serial_direct_selection"].into_iter().zip(samples) {
                        assert_eq!(samples.len(),14); let mut sorted=samples.clone(); sorted.sort_by(f64::total_cmp);
                        println!("{rows},{cols},{axis_number},{pattern},{workers},{name},14,{:.6},{:.6},{:.6},\"{:?}\"",sorted[6].midpoint(sorted[7]),sorted[0],sorted[13],samples);
                    }
                }
            }
        });
    }
}
