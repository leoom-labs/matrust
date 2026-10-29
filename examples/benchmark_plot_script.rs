use matrust::{
    Engine, files,
    plot::{Figure, PlotKind, Series},
};
use std::{path::Path, sync::atomic::AtomicBool, time::Instant};
fn main() -> Result<(), String> {
    let cancel = AtomicBool::new(false);
    if std::env::args().any(|a| a == "--bench") {
        let dir = Path::new("target/evidence/performance/data");
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let workers = rayon::current_num_threads();
        println!("workers,points,repetitions,bytes,median_ms,min_ms,max_ms");
        for n in [10_000, 100_000, 1_000_000] {
            let mut f = Figure::default();
            f.add(vec![Series::new(
                (0..n)
                    .map(|i| [i as f64 * 0.001, (i as f64 * 0.01).sin()])
                    .collect(),
                PlotKind::Line,
            )])?;
            let original = f.clone();
            let path = dir.join(format!("matlab_script_{workers}_{n}.m"));
            let mut times = Vec::new();
            for repetition in 0..6 {
                let start = Instant::now();
                files::write_matlab_plot_script_with_cancel(&path, &f, &cancel)?;
                let ms = start.elapsed().as_secs_f64() * 1000.;
                if repetition > 0 {
                    times.push(ms);
                }
            }
            assert!(original.same_snapshot(&f));
            times.sort_by(f64::total_cmp);
            let bytes = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
            println!(
                "{workers},{n},5,{bytes},{:.6},{:.6},{:.6}",
                times[2], times[0], times[4]
            );
        }
    } else {
        let dir = Path::new("target/evidence/plots/matlab-script");
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let mut engine = Engine::default();
        engine.execute("tiledlayout(2,1);nexttile;plot([1 2 3],[2 4 1],'r--o');hold on;scatter([1 2 3],[1 3 2]);stairs([1 2 3],[3 2 4]);stem([1 2 3],[4 1 3]);legend('Line','Scatter','Steps','Stems');grid on;title('Signals');xlabel('Time');ylabel('Value');nexttile;bar([1 3 5],[2 4;3 5;4 6]);legend('A','B');title('Bars');sgtitle('MATLAB script export');")?;
        let f = engine.figure.as_mut().unwrap();
        f.title = "Bars: O'Brien \u{03bb}\nLiteral labels".into();
        files::write_matlab_plot_script_with_cancel(&dir.join("all_types.m"), f, &cancel)?;
        files::save_figure(&dir.join("all_types.mfig"), f)?;
        let axes:Vec<_>=f.all_axes().map(|(_,a)| serde_json::json!({"title":a.title,"series":a.series.iter().map(|s| serde_json::json!({"kind":format!("{:?}",s.kind),"points":s.points.iter().collect::<Vec<_>>().as_slice(),"label":s.label,"bar_width":s.bar_width,"bar_offset":s.bar_offset})).collect::<Vec<_>>()})).collect();
        std::fs::write(
            dir.join("expected.json"),
            serde_json::to_vec_pretty(&axes).unwrap(),
        )
        .map_err(|e| e.to_string())?;
        println!("Wrote MATLAB script, native figure and sample metadata fixtures");
    }
    Ok(())
}
