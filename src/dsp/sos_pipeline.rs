use super::{Filter, check_cancel};
use crate::Result;
use std::sync::atomic::AtomicBool;
use wide::f64x4;

pub(super) fn process(sections: &[Filter], data: &mut [f64], cancel: &AtomicBool) -> Result<()> {
    for group in sections.chunks(4) {
        if group.len() == 4 && data.len() >= 64 && group.iter().all(|f| f.feedback) {
            quad(group, data, cancel)?;
        } else {
            for section in group {
                section.clone().process(data, cancel)?;
            }
        }
    }
    Ok(())
}

// Each lane handles one section at a different sample time. Delaying each
// subsequent lane by one iteration exposes independent recurrences to SIMD,
// without reordering the arithmetic within any section or approximating IIR.
fn quad(sections: &[Filter], data: &mut [f64], cancel: &AtomicBool) -> Result<()> {
    let b0 = f64x4::from(std::array::from_fn(|i| sections[i].b[0]));
    let b1 = f64x4::from(std::array::from_fn(|i| sections[i].b[1]));
    let b2 = f64x4::from(std::array::from_fn(|i| sections[i].b[2]));
    let a1 = f64x4::from(std::array::from_fn(|i| sections[i].a[1]));
    let a2 = f64x4::from(std::array::from_fn(|i| sections[i].a[2]));
    let mut z0 = f64x4::ZERO;
    let mut z1 = f64x4::ZERO;
    let mut previous = [0.; 4];
    check_cancel(cancel)?;
    // Fill the pipeline without advancing sections whose first input has not
    // arrived. Keeping their initial +0 state also preserves signed-zero math.
    for (i, &input) in data[..3].iter().enumerate() {
        let x = f64x4::from([input, previous[0], previous[1], previous[2]]);
        let y = b0 * x + z0;
        let active = f64x4::from(std::array::from_fn(|lane| {
            f64::from_bits(if lane <= i { u64::MAX } else { 0 })
        }));
        z0 = active.blend(z1 + b1 * x - a1 * y, z0);
        z1 = active.blend(b2 * x - a2 * y, z1);
        previous = y.to_array();
    }
    for start in (3..data.len()).step_by(1024) {
        check_cancel(cancel)?;
        for i in start..(start + 1024).min(data.len()) {
            let x = f64x4::from([data[i], previous[0], previous[1], previous[2]]);
            let y = b0 * x + z0;
            z0 = z1 + b1 * x - a1 * y;
            z1 = b2 * x - a2 * y;
            previous = y.to_array();
            data[i - 3] = previous[3];
        }
    }
    // Drain the final three valid outputs. New lane-zero inputs cannot reach
    // lane three during these iterations and need no source buffer.
    let n = data.len();
    for output in &mut data[n - 3..] {
        let x = f64x4::from([0., previous[0], previous[1], previous[2]]);
        let y = b0 * x + z0;
        z0 = z1 + b1 * x - a1 * y;
        z1 = b2 * x - a2 * y;
        previous = y.to_array();
        *output = previous[3];
    }
    Ok(())
}
