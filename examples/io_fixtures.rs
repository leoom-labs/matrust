use matrust::{Engine, files};
fn main() {
    let root = std::path::Path::new("target/evidence/io");
    std::fs::create_dir_all(root).unwrap();
    let mut e = Engine::default();
    e.execute(include_str!("signals.m")).unwrap();
    e.execute("A=[1 2;3 4]; special=[NaN Inf -Inf]; empty=zeros(0,3);")
        .unwrap();
    files::save_workspace(&root.join("rust-workspace.mat"), &e.variables).unwrap();
    let f = e.figure.as_ref().unwrap();
    files::save_figure(&root.join("signal.mfig"), f).unwrap();
    files::write_csv(&root.join("A.csv"), &e.variables["A"]).unwrap();
    #[cfg(feature = "plot-export")]
    for ext in ["png", "jpg", "svg", "pdf", "bmp", "tiff"] {
        matrust::export::export_figure(f, &root.join(format!("signal.{ext}")), 1440, 840).unwrap();
    }
}
