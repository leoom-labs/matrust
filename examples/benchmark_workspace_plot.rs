#[allow(dead_code)]
#[path = "../src/desktop/plot_request.rs"]
mod plot_request;
use matrust::{Engine, plot::PlotKind};
use std::{hint::black_box, sync::atomic::AtomicBool, time::Instant};
fn main() {
    let workers: usize = std::env::args()
        .nth(1)
        .unwrap_or("8".into())
        .parse()
        .unwrap();
    assert!([1, 8].contains(&workers));
    println!("case,workers,points,repetitions,median_ms,min_ms,max_ms");
    rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .unwrap()
        .install(|| {
            for (rows, columns) in [(1000, 1), (100000, 1), (100000, 8)] {
                let mut e = Engine::default();
                e.execute(&format!(
                    "y=reshape(1:{}, {rows}, {columns});",
                    rows * columns
                ))
                .unwrap();
                let y = e.workspace().remove("y").unwrap();
                let pointer = y.as_numeric().unwrap().data().as_ptr();
                let request = plot_request::Request {
                    x: None,
                    y: ("y".into(), y.clone()),
                    kind: PlotKind::Line,
                    log_x: false,
                    log_y: false,
                };
                assert_eq!(request.y.1.as_numeric().unwrap().data().as_ptr(), pointer);
                let cancel = AtomicBool::new(false);
                let figure = request.build(&cancel).unwrap();
                assert_eq!(figure.series.len(), columns);
                for (column, series) in figure.series.iter().enumerate() {
                    for (row, point) in series.points.iter().enumerate() {
                        assert_eq!(point, [(row + 1) as f64, (column * rows + row + 1) as f64]);
                    }
                }
                let mut times = Vec::new();
                for i in 0..14 {
                    let start = Instant::now();
                    black_box(request.build(&cancel).unwrap());
                    if i >= 3 {
                        times.push(start.elapsed().as_secs_f64() * 1000.);
                    }
                }
                times.sort_by(f64::total_cmp);
                println!(
                    "workspace_plot,{workers},{},11,{:.6},{:.6},{:.6}",
                    rows * columns,
                    times[5],
                    times[0],
                    times[10]
                );
            }
        });
}
