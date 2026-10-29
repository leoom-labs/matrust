use matrust::{Engine, files};
use std::path::Path;
fn main() {
    let mut engine = Engine::default();
    engine.execute(include_str!("log_plots.m")).unwrap();
    let figure = engine.figure.unwrap();
    let dir = Path::new("target/evidence/ui");
    std::fs::create_dir_all(dir).unwrap();
    files::save_figure(&dir.join("log-response.mfig"), &figure).unwrap();
    files::write_figure_csv(&dir.join("log-response.csv"), &figure).unwrap();
    #[cfg(feature = "plot-export")]
    for extension in ["svg", "png", "jpg", "bmp", "tiff", "pdf"] {
        matrust::export::export_figure(
            &figure,
            &dir.join(format!("log-response.{extension}")),
            1440,
            840,
        )
        .unwrap();
    }
}
