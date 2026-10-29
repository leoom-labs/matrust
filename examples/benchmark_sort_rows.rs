//! Research release comparison; no MATLAB baseline. Parent owns quiet scheduling.
use matrust::{Array, Matrix, array::MissingPlacement};
use std::{cmp::Ordering, sync::atomic::AtomicBool, time::Instant};

fn direct(a: &Matrix, keys: &[i32]) -> (Array, Option<Matrix>) {
    let rows = a.rows();
    let mut order: Vec<usize> = (0..rows).collect();
    order.sort_by(|&left, &right| {
        for &key in keys {
            let col = key.unsigned_abs() as usize - 1;
            let cmp = a.data()[col * rows + left]
                .partial_cmp(&a.data()[col * rows + right])
                .unwrap();
            let cmp = if key < 0 { cmp.reverse() } else { cmp };
            if cmp != Ordering::Equal {
                return cmp;
            }
        }
        Ordering::Equal
    });
    let mut result = Vec::with_capacity(a.len());
    for col in 0..a.cols() {
        for &row in &order {
            result.push(a.data()[col * rows + row]);
        }
    }
    (
        Matrix::from_column_vec(rows, a.cols(), result)
            .unwrap()
            .into(),
        Some(
            Matrix::from_column_vec(rows, 1, order.into_iter().map(|i| i as f64 + 1.).collect())
                .unwrap(),
        ),
    )
}
// Runtime guard permits debug compilation but rejects debug timing.
#[allow(clippy::assertions_on_constants)]
fn main() {
    assert!(!cfg!(debug_assertions), "Release only");
    let reverse = std::env::args().any(|a| a == "--reverse");
    let mut workers = [1, 8];
    if reverse {
        workers.reverse();
    }
    println!(
        "rows,cols,workers,pattern,keys,implementation,repetitions,median_ms,min_ms,max_ms,samples_ms"
    );
    for workers in workers {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        pool.install(||{
            for (rows,cols) in [(1024,4),(8193,4),(524289,2),(2,8193)] {
                for pattern in ["ordered","reverse","ties"] {
                    let matrix=Matrix::from_column_vec(rows,cols,(0..rows*cols).map(|i| match pattern {
                        "ordered"=>(i%rows) as f64,"reverse"=>(rows-i%rows) as f64,_=>((i*37)%101) as f64,
                    }).collect()).unwrap();
                    let a:Array=matrix.clone().into();let pointer=matrix.data().as_ptr();
                    for key_mode in ["all","mixed"] {
                        let keys=if key_mode=="all"{(1..=cols).map(|c|c as i32).collect::<Vec<_>>()}else{vec![cols as i32,-1]};
                        let cancel=AtomicBool::new(false);let expected=direct(&matrix,&keys);
                        let run=|candidate:bool| if candidate {a.sort_rows(Some(&keys),None,MissingPlacement::Auto,true,&cancel).unwrap()}else{direct(&matrix,&keys)};
                        let validate=|v:&(Array,Option<Matrix>)|{
                            assert_eq!(v.0.as_numeric().unwrap().data(),expected.0.as_numeric().unwrap().data());
                            assert_eq!(v.1.as_ref().unwrap().data(),expected.1.as_ref().unwrap().data());
                        };
                        for _ in 0..2{for candidate in [false,true]{validate(&run(candidate));}}
                        let mut samples=[Vec::new(),Vec::new()];
                        for _ in 0..7 {
                            for candidate in if reverse{[true,false,false,true]}else{[false,true,true,false]} {
                                let start=Instant::now();let result=run(candidate);let elapsed=start.elapsed().as_secs_f64()*1000.;
                                samples[usize::from(candidate)].push(elapsed);validate(&result);drop(result);
                            }
                        }
                        for (implementation,values) in ["direct_stable_serial","candidate"].into_iter().zip(samples) {
                            let mut sorted=values.clone();sorted.sort_by(f64::total_cmp);
                            println!("{rows},{cols},{workers},{pattern},{key_mode},{implementation},14,{:.6},{:.6},{:.6},\"{:?}\"",(sorted[6]+sorted[7])/2.,sorted[0],sorted[13],values);
                        }
                    }
                    assert_eq!(pointer,matrix.data().as_ptr());
                }
            }
        });
    }
}
