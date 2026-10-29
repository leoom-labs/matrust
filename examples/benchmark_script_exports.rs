//! Interleaved API/script exports; output checks and preparation are untimed.
use matrust::{Engine, export::export_figure_as_with_cancel};
use std::{sync::atomic::AtomicBool, time::Instant};

fn main() {
    let reverse = std::env::args().any(|a| a == "--reverse");
    let mut workers = [1, 8];
    if reverse {
        workers.reverse();
    }
    println!(
        "points,format,workers,operation,repetitions,file_bytes,median_ms,min_ms,max_ms,samples_ms"
    );
    for workers in workers {
        rayon::ThreadPoolBuilder::new().num_threads(workers).build().unwrap().install(|| {
            for points in [1000,100000] {
                for format in ["svg","png","jpeg","pdf","bmp","tiff","tiffn"] {
                    let dir = tempfile::tempdir().unwrap();
                    let path = dir.path().join("export.data");
                    let mut engine = Engine::default();
                    engine.set_file_directory(dir.path()).unwrap();
                    engine.set_function_directory(dir.path()).unwrap();
                    engine.execute(&format!("x=linspace(0,2,{points});plot(x,sin(2*pi*x));title('Export benchmark');xlabel('Time');ylabel('Value');")).unwrap();
                    let original = engine.figure.clone().unwrap();
                    let cancel = AtomicBool::new(false);
                    export_figure_as_with_cancel(&original,&path,format,1440,840,&cancel).unwrap();
                    let expected = std::fs::read(&path).unwrap();
                    let script = format!("saveas(1,'export.data','{format}');");
                    let mut operations = ["api_export","script_export"];
                    if reverse { operations.reverse(); }
                    let mut samples = [Vec::new(),Vec::new()];
                    for round in 0..9 {
                        for offset in 0..2 {
                            let index=(round+offset)%2;
                            engine.restore_figures(1,vec![(1,original.clone())]).unwrap();
                            let start=Instant::now();
                            if operations[index]=="api_export" {
                                export_figure_as_with_cancel(engine.figure.as_ref().unwrap(),&path,format,1440,840,&cancel).unwrap();
                            } else { engine.execute(&script).unwrap(); }
                            let elapsed=start.elapsed().as_secs_f64()*1000.;
                            if round>=2 {samples[index].push(elapsed);}
                            assert!(original.same_snapshot(engine.figure.as_ref().unwrap()));
                            assert!(engine.take_figure_file_events().is_empty());
                            assert_eq!(std::fs::read(&path).unwrap(),expected,"{format} output changed");
                        }
                    }
                    for (operation,samples) in operations.into_iter().zip(samples) {
                        let mut sorted=samples.clone();sorted.sort_by(f64::total_cmp);
                        println!("{points},{format},{workers},{operation},7,{},{:.6},{:.6},{:.6},\"{:?}\"",expected.len(),sorted[3],sorted[0],sorted[6],samples);
                    }
                }
            }
        });
    }
}
