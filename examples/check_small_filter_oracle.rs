use matrust::{Matrix, dsp, files};
use std::{collections::BTreeMap, path::Path, sync::atomic::AtomicBool};

fn main() -> Result<(), String> {
    let inputs = Path::new("tests/fixtures/reference/small-filter-inputs.mat");
    if std::env::args().any(|a| a == "--fixtures") {
        let mut vars = BTreeMap::new();
        for order in [2, 8, 20] {
            vars.insert(
                format!("sos{order}"),
                dsp::butter_sos(order, 0.2, dsp::Band::Lowpass)?,
            );
        }
        files::save_workspace(inputs, &vars)?;
        return Ok(());
    }
    let vars = files::load_workspace(Path::new("target/evidence/small-filter-oracle.mat"))?;
    let coefficients = files::load_workspace(inputs)?;
    let cancel = AtomicBool::new(false);
    for threads in [1, 8] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .map_err(|e| e.to_string())?;
        pool.install(|| -> Result<(),String> {
            let mut count=0;
            let mut max=0f64;
            let mut compare=|name:&str,actual:&Matrix| -> Result<(),String> {
                let expected=&vars[name];
                if (actual.rows(),actual.cols())!=(expected.rows(),expected.cols()) { return Err(format!("{name}: shape mismatch")); }
                for (&a,&b) in actual.data().iter().zip(expected.data()) {
                    let err=(a-b).abs();
                    if !err.is_finite() || err>2e-13*(1.+b.abs()) { return Err(format!("{name}: {a} != {b}")); }
                    max=max.max(err); count+=1;
                }
                Ok(())
            };
            for order in [2,8,20] {
                compare(&format!("y_sos{order}"),&dsp::sosfilt(&coefficients[&format!("sos{order}")],&vars["x"],&cancel)?)?;
            }
            for k in 1..=4 {
                let (b,a)=match k {
                    1=>(vec![0.2,0.3],vec![2.]),
                    2=>(vec![0.2,0.3,0.4],vec![2.]),
                    3=>(vec![0.2,0.3],vec![2.,-0.7]),
                    _=>(vec![0.2,0.3,0.4],vec![2.,-0.7,0.2]),
                };
                let zi=Matrix::filled(b.len()-1,8,0.125)?;
                let (y,z)=dsp::filter_with_state(&b,&a,&vars["x"],Some(&zi),Some(1),&cancel)?;
                compare(&format!("y{k}"),&y)?; compare(&format!("z{k}"),&z)?;
                let (y,z)=dsp::filter_with_state(&b,&a,&vars["x"].transpose(),Some(&zi),Some(2),&cancel)?;
                compare(&format!("y{k}"),&y.transpose())?; compare(&format!("z{k}"),&z)?;
            }
            println!("{threads} workers: {count} MATLAB filter values passed; max absolute error {max:.3e}");
            Ok(())
        })?;
    }
    Ok(())
}
