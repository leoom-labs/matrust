use matrust::{Engine, files};
fn main() {
    let mut e = Engine::default();
    e.execute(include_str!("logical_selection.m")).unwrap();
    e.execute("signed=[-0 NaN Inf -Inf]; empty=false(0,3); tiny=true;")
        .unwrap();
    std::fs::create_dir_all("target/evidence/logical").unwrap();
    files::save_array_workspace(
        std::path::Path::new("target/evidence/logical/rust-workspace.mat"),
        &e.workspace(),
        &e.cancellation(),
    )
    .unwrap();
    files::save_array_workspace_uncompressed(
        std::path::Path::new("target/evidence/logical/rust-workspace-uncompressed.mat"),
        &e.workspace(),
        &e.cancellation(),
    )
    .unwrap();
    files::write_array_csv(
        std::path::Path::new("target/evidence/logical/rust-mask.csv"),
        &e.array("selected").unwrap(),
        &e.cancellation(),
    )
    .unwrap();
    println!(
        "MATRUST_LOGICAL_EXPORT_OK: {} variables, {} selected samples",
        e.workspace().len(),
        e.variables["selected_count"].data()[0]
    );
}
