use matrust::{Engine, files, plot::LegendLocation};
use std::{path::Path, sync::atomic::AtomicBool};
fn save(figure: &matrust::plot::Figure, dir: &Path, name: &str) {
    files::save_figure(&dir.join(format!("{name}.mfig")), figure).unwrap();
    std::fs::write(dir.join(format!("{name}.svg")), figure.svg()).unwrap();
    std::fs::write(
        dir.join(format!("{name}.m")),
        files::matlab_plot_script(figure, &AtomicBool::new(false)).unwrap(),
    )
    .unwrap();
    #[cfg(feature = "plot-export")]
    for ext in ["png", "pdf"] {
        matrust::export::export_figure(figure, &dir.join(format!("{name}.{ext}")), 960, 560)
            .unwrap();
    }
}
fn main() {
    let dir = Path::new("target/evidence/plots/legend-location");
    std::fs::create_dir_all(dir).unwrap();
    let mut engine = Engine::default();
    engine.execute("plot([1 2 3 4],[1 4 2 3],'b-o');axis([0 5 0 5]);legend('Signal');title('Legend positions');").unwrap();
    for location in LegendLocation::ALL {
        let figure = engine.figure.as_mut().unwrap();
        figure.legend_location = location;
        save(figure, dir, location.name());
    }
    let hidden = engine.figure.as_mut().unwrap();
    hidden.legend = false;
    hidden.legend_location = LegendLocation::Southwest;
    save(hidden, dir, "hidden");
    engine.execute(include_str!("legend_locations.m")).unwrap();
    save(engine.figure.as_ref().unwrap(), dir, "layout");
    println!("Exported all eight legend locations and four-panel layout.");
}
