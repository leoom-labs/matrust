use matrust::{files, plot::PlotKind};
use std::path::Path;
fn main() {
    std::fs::create_dir_all("target/evidence/ui").unwrap();
    std::fs::create_dir_all("target/evidence/figures").unwrap();
    let mut f = files::load_figure(Path::new("tests/fixtures/fig/matlab-hidden.fig")).unwrap();
    files::save_figure(Path::new("target/evidence/ui/hidden.mfig"), &f).unwrap();
    files::write_figure_csv(Path::new("target/evidence/ui/hidden.csv"), &f).unwrap();
    #[cfg(feature = "plot-export")]
    for extension in ["svg", "png", "jpg", "bmp", "tiff"] {
        matrust::export::export_figure(
            &f,
            &Path::new("target/evidence/ui").join(format!("hidden.{extension}")),
            1440,
            840,
        )
        .unwrap();
    }
    #[cfg(feature = "plot-export")]
    {
        std::fs::create_dir_all("output/pdf").unwrap();
        matrust::export::export_figure(&f, Path::new("output/pdf/hidden.pdf"), 1440, 840).unwrap();
    }
    f.legend = false;
    files::save_figure(Path::new("target/evidence/figures/rust-hidden.fig"), &f).unwrap();
    for s in &mut f.series {
        s.visible = true;
    }
    files::save_figure(Path::new("target/evidence/ui/restored.mfig"), &f).unwrap();
    assert!(f.series.iter().all(|s| s.kind == PlotKind::Line));
    println!("Visibility export fixtures generated; hidden samples preserved in mfig/CSV/FIG");
}
