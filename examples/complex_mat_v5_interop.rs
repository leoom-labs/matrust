//! Parent-owned interoperability probe, not a runtime application dependency.
use matrust::files;
use std::{path::PathBuf, sync::atomic::AtomicBool};

fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    assert_eq!(
        args.len(),
        2,
        "Usage: complex_mat_v5_interop INPUT.mat FRESH_OUTPUT_DIRECTORY"
    );
    let input = PathBuf::from(&args[0]);
    let output = PathBuf::from(&args[1]);
    std::fs::create_dir(&output).expect("Fresh output directory required");
    let cancel = AtomicBool::new(false);
    let catalog = files::inspect_workspace(&input, &cancel).unwrap();
    let names = catalog.variables().map(|v| v.name.clone()).collect();
    let values = files::load_selected_arrays(&catalog, &names, &cancel).unwrap();
    files::save_array_workspace_uncompressed(&output.join("matrust-plain.mat"), &values, &cancel)
        .unwrap();
    files::save_array_workspace(&output.join("matrust-compressed.mat"), &values, &cancel).unwrap();
    let metadata:Vec<_>=catalog.variables().map(|v|serde_json::json!({"name":v.name,"shape":v.dimensions,"class":v.class,"complex":v.complex,"storage_bytes":v.storage_bytes()})).collect();
    std::fs::write(
        output.join("metadata.json"),
        serde_json::to_vec_pretty(&metadata).unwrap(),
    )
    .unwrap();
    println!("COMPLEX_MAT_V5_INTEROP_COMPLETE variables={}", values.len());
}
