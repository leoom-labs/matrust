//! Proposed release benchmark; parent owns hardware/hash/raw process recording.
use matrust::{
    Matrix,
    matrix::{Dispersion, StatisticsAxis},
};
use std::{sync::atomic::AtomicBool, time::Instant};
fn main() {
    if cfg!(debug_assertions) {
        panic!("Use a release build");
    }
    let reverse = std::env::args().any(|arg| arg == "--reverse");
    let mut workers = [1, 8];
    if reverse {
        workers.reverse();
    }
    println!("rows,cols,axis,kind,workers,repetitions,median_us,min_us,max_us,samples_us");
    for workers in workers {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        pool.install(|| {
            for (rows,cols) in [(64,8),(65536,1),(65536,8),(262144,8),(8,262144)] {
                let input=Matrix::from_column_vec(rows,cols,(0..rows*cols).map(|i|2.0f64.powi(40)+(i%67) as f64/8.).collect()).unwrap();
                let mut axes=[("columns",StatisticsAxis::Dimension(1)),("rows",StatisticsAxis::Dimension(2)),("all",StatisticsAxis::All)];
                if reverse {axes.reverse();}
                for (axis_name,axis) in axes {
                    for (name,kind) in [("var",Dispersion::Variance),("std",Dispersion::StandardDeviation)] {
                        let cancel=AtomicBool::new(false);
                        let expected=input.dispersion(kind,false,axis,false,&cancel).unwrap();
                        let mut samples=Vec::new();
                        for iteration in 0..9 {
                            let start=Instant::now();
                            let value=input.dispersion(kind,false,axis,false,&cancel).unwrap();
                            let elapsed=start.elapsed().as_secs_f64()*1e6;
                            if iteration>=2 {samples.push(elapsed);}
                            assert_eq!(value.data().iter().map(|v|v.to_bits()).collect::<Vec<_>>(),expected.data().iter().map(|v|v.to_bits()).collect::<Vec<_>>());
                            drop(value);
                        }
                        let mut sorted=samples.clone(); sorted.sort_by(f64::total_cmp);
                        println!("{rows},{cols},{axis_name},{name},{workers},7,{:.6},{:.6},{:.6},\"{:?}\"",sorted[3],sorted[0],sorted[6],samples);
                    }
                }
            }
        });
    }
}
