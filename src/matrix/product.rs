//! Narrow checked adapter to the runtime-dispatched f64 GEMM backend.
use super::Matrix;

pub(super) fn multiply(a: &Matrix, b: &Matrix, output: &mut [f64], workers: usize) {
    // These inexpensive checks make the raw-pointer contract explicit even if this
    // private adapter gains another caller. Matrix::mul checks allocation bounds first.
    assert_eq!(a.cols(), b.rows());
    assert_eq!(output.len(), a.rows() * b.cols());
    assert!(workers > 0);
    let parallel = if workers > 1 {
        gemm::Parallelism::Rayon(workers)
    } else {
        gemm::Parallelism::None
    };
    // SAFETY: Matrix owns valid contiguous column-major f64 storage. The checked
    // dimensions and element strides describe exactly these input/output slices.
    // The fresh mutable output cannot alias either shared immutable input. All
    // dimensions are bounded by MAX_ELEMENTS, hence strides fit isize. The backend
    // completes all Rayon jobs before returning; the slices remain live throughout.
    unsafe {
        gemm::gemm(
            a.rows(),
            b.cols(),
            a.cols(),
            output.as_mut_ptr(),
            a.rows() as isize,
            1,
            false,
            a.data().as_ptr(),
            a.rows() as isize,
            1,
            b.data().as_ptr(),
            b.rows() as isize,
            1,
            0.,
            1.,
            false,
            false,
            false,
            parallel,
        );
    }
}
