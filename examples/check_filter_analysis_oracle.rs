use matrust::{
    dsp::{self, Analysis, Band},
    files,
};
use std::{path::Path, sync::atomic::AtomicBool};
fn main() -> Result<(), String> {
    let vars = files::load_workspace(Path::new(
        "tests/fixtures/reference/filter-analysis-oracle.mat",
    ))?;
    let cancel = AtomicBool::new(false);
    let mut maximum = 0.0f64;
    let mut count = 0;
    let mut compare = |name: &str, actual: Vec<f64>| -> Result<(), String> {
        let expected = vars[name].data();
        if actual.len() != expected.len() {
            return Err(format!("{name}: length mismatch"));
        }
        let mut local_max = 0.0f64;
        for (a, b) in actual.iter().zip(expected) {
            let error = (a - b).abs();
            if !error.is_finite() || error > 2e-8 * (1. + b.abs()) {
                return Err(format!("{name}: Rust {a}, MATLAB {b}"));
            }
            local_max = local_max.max(error);
            maximum = maximum.max(error);
            count += 1;
        }
        println!("{name}: {} values, max error {local_max:.3e}", actual.len());
        Ok(())
    };
    for (kind, suffix, n) in [
        (Analysis::Phase, "phase", 512),
        (Analysis::GroupDelay, "delay", 512),
        (Analysis::Impulse, "impulse", 256),
        (Analysis::Step, "step", 256),
    ] {
        compare(
            &format!("analysis_{suffix}"),
            dsp::analyze(kind, &[0.2, 0.3, 0.2], &[2., -0.7, 0.2], n, &cancel)?,
        )?;
        compare(
            &format!("sos_{suffix}"),
            dsp::analyze_sos(kind, &vars["analysis_sos"], n, &cancel)?,
        )?;
    }
    compare(
        "delay_phase",
        dsp::analyze(Analysis::Phase, &[0., 0., 1.], &[1.], 512, &cancel)?,
    )?;
    compare(
        "fir_delay",
        dsp::analyze(
            Analysis::GroupDelay,
            &dsp::fir1(64, &[0.2], Band::Lowpass)?,
            &[1.],
            512,
            &cancel,
        )?,
    )?;
    println!(
        "Filter analysis reference cases: {count} values passed; max absolute error {maximum:.3e}"
    );
    Ok(())
}
