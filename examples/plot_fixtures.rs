#[cfg(feature = "plot-export")]
fn main() -> Result<(), String> {
    let dir = std::path::Path::new("target/evidence/plots");
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut engine = matrust::Engine::default();
    engine.execute(include_str!("multiple_plots.m"))?;
    for (id, name) in [(1, "signals"), (2, "bars")] {
        engine.select_figure(id)?;
        let f = engine.figure.as_ref().unwrap();
        matrust::files::save_figure(&dir.join(format!("{name}.mfig")), f)?;
        let restored = matrust::files::load_figure(&dir.join(format!("{name}.mfig")))?;
        assert_eq!(f.svg(), restored.svg());
        for ext in ["png", "jpg", "svg", "pdf", "bmp", "tiff"] {
            matrust::export::export_figure(
                &restored,
                &dir.join(format!("{name}.{ext}")),
                1440,
                840,
            )?;
        }
        matrust::files::write_figure_csv(&dir.join(format!("{name}.csv")), &restored)?;
    }
    println!("Verified editable fixtures and exported both figures to target/evidence/plots");
    Ok(())
}
#[cfg(not(feature = "plot-export"))]
fn main() {
    eprintln!("Run with --features plot-export");
}
