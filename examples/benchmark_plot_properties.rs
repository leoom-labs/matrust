use matrust::Engine;
use std::{hint::black_box, time::Instant};

fn main() {
    let workers: usize = std::env::args()
        .nth(1)
        .unwrap_or("1".into())
        .parse()
        .unwrap();
    assert!([1, 8].contains(&workers));
    println!("case,points,series,workers,iterations,repetitions,median_ms,min_ms,max_ms");
    rayon::ThreadPoolBuilder::new().num_threads(workers).build().unwrap().install(|| {
        for (rows,columns,iterations) in [(16,1,200),(100000,8,1)] {
            let mut engine=Engine::default();
            engine.execute(&format!("x=(1:{rows})';Y=ones({rows},{columns});")).unwrap();
            for (case,source) in [
                ("plain","plot(x,Y);"),
                ("properties","plot(x,Y,'LineWidth',2,'Color',[0.1 0.5 0.9],'LineStyle','--','DisplayName','Signal');"),
            ] {
                let mut times=Vec::new();
                for sample in 0..14 {
                    let start=Instant::now();
                    for _ in 0..iterations { black_box(engine.execute(source).unwrap()); }
                    if sample>=3 { times.push(start.elapsed().as_secs_f64()*1000./iterations as f64); }
                }
                let f=engine.figure.as_ref().unwrap();
                assert_eq!(f.point_count(),rows*columns);
                assert_eq!(f.series.len(),columns);
                assert_eq!(f.series[columns-1].points.get(rows-1).unwrap(),[rows as f64,1.]);
                assert_eq!(f.series[0].line_width,if case=="properties" {Some(2.)} else {None});
                times.sort_by(f64::total_cmp);
                println!("{case},{},{columns},{workers},{iterations},11,{:.6},{:.6},{:.6}",rows*columns,times[5],times[0],times[10]);
            }
        }
    });
    if workers == 1 {
        let dir = std::path::Path::new("target/evidence/plots/plot-properties");
        std::fs::create_dir_all(dir).unwrap();
        let source = include_str!("plot_properties.m");
        std::fs::write(dir.join("source.m"), source).unwrap();
        let mut engine = Engine::default();
        engine.execute(source).unwrap();
        let mut figure = engine.figure.unwrap();
        matrust::files::save_figure(&dir.join("properties.mfig"), &figure).unwrap();
        std::fs::write(dir.join("properties.svg"), figure.svg()).unwrap();
        #[cfg(feature = "plot-export")]
        for ext in ["png", "pdf"] {
            matrust::export::export_figure(
                &figure,
                &dir.join(format!("properties.{ext}")),
                960,
                560,
            )
            .unwrap();
        }
        figure.legend = false;
        matrust::files::save_figure(&dir.join("properties.fig"), &figure).unwrap();
    }
}
