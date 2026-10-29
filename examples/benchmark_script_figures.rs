//! Interleaved file API and script figure I/O; preparation/checks exclude timing.
use matrust::{Engine, files::figure_version, plot::Figure};
use std::{sync::atomic::AtomicBool, time::Instant};

fn verify(actual: &Figure, expected: &Figure) {
    assert_eq!(actual.title, expected.title);
    assert_eq!(actual.xlabel, expected.xlabel);
    assert_eq!(actual.ylabel, expected.ylabel);
    assert_eq!(actual.series.len(), expected.series.len());
    for (a, b) in actual.series.iter().zip(&expected.series) {
        assert_eq!(a.points.len(), b.points.len());
        assert!(
            a.points
                .iter()
                .zip(b.points.iter())
                .all(|(a, b)| a[0].to_bits() == b[0].to_bits() && a[1].to_bits() == b[1].to_bits())
        );
    }
}

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
                for format in ["mfig","fig"] {
                    let dir = tempfile::tempdir().unwrap();
                    let filename = format!("data.{format}");
                    let path = dir.path().join(&filename);
                    let mut engine = Engine::default();
                    engine.set_file_directory(dir.path()).unwrap();
                    engine.set_function_directory(dir.path()).unwrap();
                    engine.execute(&format!("x=linspace(0,2,{points});plot(x,sin(2*pi*x));title('file benchmark');xlabel('x');ylabel('y');")).unwrap();
                    let expected = engine.figure.clone().unwrap();
                    let cancel = AtomicBool::new(false);
                    figure_version::save(&path,&expected,None,&cancel).unwrap();
                    let save = format!("savefig('{filename}');");
                    let open = format!("openfig('{filename}');");
                    let mut operations = ["api_save","script_save","api_open","script_open"];
                    if reverse { operations.reverse(); }
                    let mut samples = vec![Vec::new();operations.len()];
                    for round in 0..9 {
                        for offset in 0..operations.len() {
                            let index = (round+offset)%operations.len();
                            let operation = operations[index];
                            // Both open workflows start with no live figure;
                            // both saves use the same shared source snapshot.
                            engine.restore_figures(1, if operation.ends_with("save") { vec![(1,expected.clone())] } else { vec![] }).unwrap();
                            let start = Instant::now();
                            match operation {
                                "api_save" => { figure_version::save(&path,engine.figure.as_ref().unwrap(),None,&cancel).unwrap(); }
                                "script_save" => { engine.execute(&save).unwrap(); }
                                "api_open" => {
                                    let (figure,_) = figure_version::load(&path,&cancel).unwrap();
                                    engine.restore_figures(1,vec![(1,figure)]).unwrap();
                                }
                                "script_open" => { engine.execute(&open).unwrap(); }
                                _ => unreachable!(),
                            }
                            let elapsed = start.elapsed().as_secs_f64()*1000.;
                            if round>=2 { samples[index].push(elapsed); }
                            verify(engine.figure.as_ref().unwrap(),&expected);
                            let (loaded,_) = figure_version::load(&path,&cancel).unwrap();
                            verify(&loaded,&expected);
                            engine.take_figure_file_events();
                        }
                    }
                    for (operation,samples) in operations.into_iter().zip(samples) {
                        let mut sorted = samples.clone();sorted.sort_by(f64::total_cmp);
                        println!("{points},{format},{workers},{operation},7,{},{:.6},{:.6},{:.6},\"{:?}\"",path.metadata().unwrap().len(),sorted[3],sorted[0],sorted[6],samples);
                    }
                }
            }
        });
    }
}
