//! Draft release harness. Interleave API/script on1/8workers; outputs checked untimed.
#[cfg(not(feature = "plot-export"))]
fn main() {
    eprintln!("Use --features plot-export");
}
#[cfg(feature = "plot-export")]
fn main() {
    use matrust::{
        Engine,
        export::{GraphicsOptions, export_graphics_with_cancel},
    };
    use std::{sync::atomic::AtomicBool, time::Instant};
    let reverse = std::env::args().any(|a| a == "--reverse");
    let mut workers = [1, 8];
    if reverse {
        workers.reverse();
    }
    println!("points,format,dpi,workers,operation,repetitions,bytes,median_ms,samples_ms");
    for workers in workers {
        rayon::ThreadPoolBuilder::new().num_threads(workers).build().unwrap().install(|| {
            for points in [1000,100000] {for format in ["svg","png","jpeg","tiff","pdf"] {for dpi in [96,300] {
                let dir=tempfile::tempdir().unwrap();let path=dir.path().join(format!("output.{format}"));
                let mut engine=Engine::default();engine.set_file_directory(dir.path()).unwrap();engine.set_function_directory(dir.path()).unwrap();
                engine.execute(&format!("t=linspace(0,1,{points});plot(t,sin(20*t));title('Tight export benchmark');xlabel('Time');ylabel('Signal');")).unwrap();
                let figure=engine.figure.clone().unwrap();let cancel=AtomicBool::new(false);
                export_graphics_with_cancel(&figure,&path,format,GraphicsOptions {resolution:dpi,transparent:false},&cancel).unwrap();
                let expected=std::fs::read(&path).unwrap();let script=format!("exportgraphics(1,'output.{format}','Resolution',{dpi});");
                let mut names=["api","script"];if reverse {names.reverse();}let mut samples=[Vec::new(),Vec::new()];
                for round in 0..9 {for offset in 0..2 {
                    let index=(round+offset)%2;let start=Instant::now();
                    if names[index]=="api" {export_graphics_with_cancel(&figure,&path,format,GraphicsOptions {resolution:dpi,transparent:false},&cancel).unwrap();}
                    else {engine.execute(&script).unwrap();}
                    let elapsed=start.elapsed().as_secs_f64()*1000.;if round>=2 {samples[index].push(elapsed);}
                    assert_eq!(std::fs::read(&path).unwrap(),expected);assert!(figure.same_snapshot(engine.figure.as_ref().unwrap()));
                }}
                for (name,samples) in names.into_iter().zip(samples) {let mut sorted=samples.clone();sorted.sort_by(f64::total_cmp);println!("{points},{format},{dpi},{workers},{name},7,{},{},\"{:?}\"",expected.len(),sorted[3],samples);}
            }}}
        });
    }
}
