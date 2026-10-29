use matrust::{Engine, files};
use std::path::Path;
fn main() {
    std::fs::create_dir_all("target/evidence/figures").unwrap();
    for name in [
        "matlab-lines.fig",
        "matlab-lines-v6.fig",
        "matlab-log-scatter.fig",
        "matlab-bars.fig",
        "matlab-bar-spacing-1.fig",
        "matlab-bar-spacing-2.fig",
        "matlab-bar-spacing-3.fig",
    ] {
        let path = Path::new("tests/fixtures/fig").join(name);
        let f = files::load_matlab_figure(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
        println!(
            "{name}: {} series, {} points, title={}, labels={}/{}, legend={}",
            f.series.len(),
            f.point_count(),
            f.title,
            f.xlabel,
            f.ylabel,
            f.legend
        );
        let output = Path::new("target/evidence/figures").join(name);
        files::save_figure(&output.with_extension("mfig"), &f).unwrap();
        #[cfg(feature = "plot-export")]
        if name.starts_with("matlab-bar-spacing-") {
            matrust::export::export_figure(&f, &output.with_extension("png"), 960, 650).unwrap();
        }
    }
    assert!(files::load_matlab_figure(Path::new("tests/fixtures/fig/matlab-surface.fig")).is_err());
    assert_eq!(
        files::load_matlab_figure(Path::new("tests/fixtures/fig/matlab-subplots.fig"))
            .unwrap()
            .all_axes()
            .count(),
        2
    );
    let mut engine = Engine::default();
    engine.execute("semilogx([1 10 100],[1 2 4],'r--o');hold on;plot([1 10 100],[3 4 6],'b:');title('Rust figure');xlabel('Time');ylabel('Amplitude');axis([1 100 0 8]);grid on;").unwrap();
    files::save_matlab_figure(
        Path::new("target/evidence/figures/rust-lines.fig"),
        engine.figure.as_ref().unwrap(),
    )
    .unwrap();
    println!("Wrote Rust FIG for independent MATLAB openfig validation");
    engine.execute("legend('First','Second');").unwrap();
    assert!(
        files::save_matlab_figure(
            Path::new("target/evidence/figures/rejected-legend.fig"),
            engine.figure.as_ref().unwrap()
        )
        .is_err()
    );
    engine
        .execute("hold off;bar([1 2 3],[2 4;3 5;4 6]);")
        .unwrap();
    assert!(
        files::save_matlab_figure(
            Path::new("target/evidence/figures/rejected-bars.fig"),
            engine.figure.as_ref().unwrap()
        )
        .is_err()
    );
    engine
        .execute("hold off;scatter([1 2 3],[4 5 6],'r');title('Points');")
        .unwrap();
    files::save_matlab_figure(
        Path::new("target/evidence/figures/rust-scatter.fig"),
        engine.figure.as_ref().unwrap(),
    )
    .unwrap();
}
