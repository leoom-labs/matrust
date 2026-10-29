#[cfg(feature = "plot-export")]
fn main() {
    use matrust::{Engine, export, files};
    use std::{path::PathBuf, sync::atomic::AtomicBool};
    let dir = PathBuf::from(std::env::args().nth(1).expect("fixture directory"));
    std::fs::create_dir_all(&dir).unwrap();
    let mut engine = Engine::default();
    engine.execute("plot([0 1 2 3 4],[1 2 1 2 1],'bo-');hold on;plot([0 1 2 3 4],[3 4 3 4 3],'ro-');plot([0 1 2 3 4],[5 6 5 6 5],'go-');title('Line widths preserved across save and open');xlabel('Sample');ylabel('Amplitude');legend('0.75 pt','2 pt','6 pt');axis([0 4 0 7]);").unwrap();
    let mut figure = engine.figure.unwrap();
    for (series, width) in figure.series.iter_mut().zip([0.75, 2., 6.]) {
        series.line_width = Some(width);
    }
    files::save_figure(&dir.join("widths.mfig"), &figure).unwrap();
    for ext in ["svg", "png", "pdf"] {
        export::export_figure(&figure, &dir.join(format!("widths.{ext}")), 960, 560).unwrap();
    }
    std::fs::write(
        dir.join("widths.m"),
        files::matlab_plot_script(&figure, &AtomicBool::new(false)).unwrap(),
    )
    .unwrap();
    figure.legend = false;
    files::save_figure(&dir.join("widths.fig"), &figure).unwrap();
    let reopened = files::load_figure(&dir.join("widths.fig")).unwrap();
    assert_eq!(figure.svg(), reopened.svg());
    println!("LINE_WIDTH_FIXTURE_OK");
}
#[cfg(not(feature = "plot-export"))]
fn main() {
    eprintln!("Enable plot-export to create the line-width export fixtures");
}
