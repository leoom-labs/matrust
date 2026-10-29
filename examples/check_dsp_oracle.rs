use matrust::{
    Matrix,
    dsp::{self, Band},
    files,
};
use std::{path::Path, sync::atomic::AtomicBool};
fn main() -> Result<(), String> {
    let vars = files::load_workspace(Path::new("tests/fixtures/reference/dsp-oracle.mat"))?;
    let cancel = AtomicBool::new(false);
    let mut max_error = 0.0f64;
    let mut comparisons = 0;
    let mut compare = |name: &str, actual: &[f64], expected: &[f64]| -> Result<(), String> {
        if actual.len() != expected.len() {
            return Err(format!("{name}: shape mismatch"));
        }
        for (a, b) in actual.iter().zip(expected) {
            let e = (a - b).abs();
            if !e.is_finite() || e > 3e-11 * (1. + b.abs()) {
                return Err(format!("{name}: {a} differs from MATLAB {b}"));
            }
            max_error = max_error.max(e);
            comparisons += 1;
        }
        Ok(())
    };
    for (name, order, cuts, band) in [
        ("fir_low", 32, vec![0.2], Band::Lowpass),
        ("fir_high", 31, vec![0.4], Band::Highpass),
        ("fir_band", 32, vec![0.2, 0.5], Band::Bandpass),
        ("fir_stop", 32, vec![0.2, 0.5], Band::Bandstop),
    ] {
        compare(name, &dsp::fir1(order, &cuts, band)?, vars[name].data())?;
    }
    compare(
        "filter",
        dsp::filter(&[0.2, 0.3, 0.2], &[2., -0.7, 0.2], &vars["input"], &cancel)?.data(),
        vars["filter_output"].data(),
    )?;
    let h = dsp::response(
        vars["fir_low"].data(),
        &[1.],
        &(0..512).map(|i| i as f64 / 512.).collect::<Vec<_>>(),
        &cancel,
    )?;
    compare(
        "freqz real",
        &h.iter().map(|v| v[0]).collect::<Vec<_>>(),
        vars["response_real"].data(),
    )?;
    compare(
        "freqz imag",
        &h.iter().map(|v| v[1]).collect::<Vec<_>>(),
        vars["response_imag"].data(),
    )?;
    let mut impulse = vec![0.; 256];
    impulse[0] = 1.;
    let x = Matrix::from_column_slice(256, 1, &impulse)?;
    for i in 0..20 {
        let sos = dsp::butter_sos(
            i % 10 + 1,
            0.2,
            if i < 10 {
                Band::Lowpass
            } else {
                Band::Highpass
            },
        )?;
        compare(
            "butterworth impulse",
            dsp::sosfilt(&sos, &x, &cancel)?.data(),
            &vars["butter_impulses"].data()[i * 256..(i + 1) * 256],
        )?;
    }
    println!(
        "DSP reference cases: {comparisons} values passed; max absolute error {max_error:.3e}"
    );
    Ok(())
}
