use matrust::{Engine, files};
use std::path::Path;
fn main() -> Result<(), String> {
    let expected = files::load_workspace(Path::new(
        "tests/fixtures/reference/filter-state-oracle.mat",
    ))?;
    let mut engine = Engine::default();
    engine.execute(
        "b=[0.2 0.3 0.2]; a=[2 -0.7 0.2]; x=reshape(sin(1:96),32,3);
[y_columns,z_columns]=filter(b,a,x,[0.1;0.2]);
[y_per_channel,z_per_channel]=filter(b,a,x,[0.1 0.2 0.3;0.4 0.5 0.6]);
[y_row,z_row]=filter(b,a,x(:,1)',[0.1 0.2]);
[y_rows,z_rows]=filter(b,a,x',[0.1 0.2 0.3;0.4 0.5 0.6],2);
[y_row_dim1,z_row_dim1]=filter(b,a,[1 2 3],[],1);
[y_col_dim2,z_col_dim2]=filter(b,a,[1;2;3],[],2);
[y_empty,z_empty]=filter(b,a,zeros(0,3),[0.1;0.2]);
[y_empty_row,z_empty_row]=filter(b,a,zeros(1,0),[0.1;0.2]);
[y_empty_channels,z_empty_channels]=filter(b,a,zeros(3,0));
[y_scalar,z_scalar]=filter(b,a,2,[0.1;0.2]);
[y_order0,z_order0]=filter(3,2,x);
[y_chunk1,z_chunk1]=filter(b,a,x(1:11,:),[0.1;0.2]);
[y_chunk2,z_chunk2]=filter(b,a,x(12:end,:),z_chunk1);
[y_fir_nan,z_fir_nan]=filter([1 2 3],1,[1 NaN 3 4 5 6 7]);
[y_fir_zi_nan,z_fir_zi_nan]=filter([1 2 3],1,[1 2 3 4 5],[NaN;0]);
[y_iir_zi_nan,z_iir_zi_nan]=filter(b,a,[1 2 3 4],[NaN;0]);",
    )?;
    let mut count = 0;
    let mut max = 0f64;
    for (name, actual) in &engine.variables {
        let expected = expected
            .get(name)
            .ok_or(format!("No MATLAB result for {name}"))?;
        if (actual.rows(), actual.cols()) != (expected.rows(), expected.cols()) {
            return Err(format!("{name}: shape mismatch"));
        }
        for (&a, &b) in actual.data().iter().zip(expected.data()) {
            if (a.is_nan() && b.is_nan()) || (a.is_infinite() && a == b) {
                count += 1;
                continue;
            }
            let error = (a - b).abs();
            if !error.is_finite() || error > 3e-13 * (1. + b.abs()) {
                return Err(format!("{name}: {a} differs from MATLAB {b}"));
            }
            max = max.max(error);
            count += 1;
        }
    }
    println!(
        "Filter-state reference cases: {count} values passed; maximum absolute error {max:.3e}"
    );
    Ok(())
}
