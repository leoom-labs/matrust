//! Representative corrected N-D value paths; release measurements, no baseline
//! comparison because the old typed paths computed different classes/values.
#[path = "nd/affinity.rs"]
mod affinity;
use matrust::Engine;
use std::{hint::black_box, time::Instant};
fn main() {
    let workers = std::thread::available_parallelism()
        .map_or(1, usize::from)
        .min(4);
    println!("shape=64x64x64,repetitions=11,warmup=3,hardware=i7-12700KF");
    println!(
        "storage_bytes: Matrix={} Logical={} Complex={} Typed={} Character={} Array={}",
        std::mem::size_of::<matrust::Matrix>(),
        std::mem::size_of::<matrust::LogicalMatrix>(),
        std::mem::size_of::<matrust::ComplexMatrix>(),
        std::mem::size_of::<matrust::TypedArray>(),
        std::mem::size_of::<matrust::CharacterRow>(),
        std::mem::size_of::<matrust::Array>()
    );
    println!("operation,workers,median_ms,min_ms,max_ms");
    for threads in [1, workers] {
        rayon::ThreadPoolBuilder::new().start_handler(affinity::bind_worker).num_threads(threads).build().unwrap().install(|| {
            for (label,statement) in [
                ("typed implicit addition", "C=A+0.6;"),
                ("typed extrema axis3", "[M,I]=max(A,[],3);"),
                ("typed cat axis4", "C=cat(4,A,A);"),
                ("200 typed scalar writes", "for k=1:200; A(k)=a; end;"),
                ("single sum axis2", "C=sum(S,2);"),
                ("complex log", "C=log(-D);"),
            ] {
                let mut e=Engine::default();
                e.execute("A=ones(64,64,64,'int64'); a=intmax('int64')-5; S=single(ones(64,64,64)); D=ones(64,64,64);").unwrap();
                for _ in 0..3 {black_box(e.execute(statement).unwrap());}
                let mut samples=Vec::with_capacity(11);
                for _ in 0..11 {
                    let start=Instant::now(); black_box(e.execute(black_box(statement)).unwrap());
                    samples.push(start.elapsed().as_secs_f64()*1000.);
                }
                samples.sort_by(f64::total_cmp);
                println!("{label},{threads},{:.4},{:.4},{:.4}",samples[5],samples[0],samples[10]);
            }
        });
    }
}
