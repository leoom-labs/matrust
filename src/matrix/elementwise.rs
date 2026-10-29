//! Specialized element-wise kernels: portable vector math with accurate fallbacks.
use super::{Matrix, PAR_ELEMENTS};
use rayon::prelude::*;
use wide::f64x4;

const CHUNK: usize = 8192;

impl Matrix {
    pub fn sin(&self) -> Self {
        self.vector_map(libm::sin, f64x4::sin, |x| x.abs() <= 8192.)
    }
    pub fn cos(&self) -> Self {
        self.vector_map(libm::cos, f64x4::cos, |x| x.abs() <= 8192.)
    }
    pub fn tan(&self) -> Self {
        self.vector_map(libm::tan, vector_tan, |x| x.abs() <= 8192.)
    }
    pub fn exp(&self) -> Self {
        // wide's exp clamps outside its approximation range. The narrower guard
        // ensures overflow, underflow and subnormal outputs use full scalar libm.
        self.vector_map(libm::exp, f64x4::exp, |x| x.abs() <= 700.)
    }
    pub fn ln(&self) -> Self {
        // wide 0.7's logarithm loses precision on ordinary finite inputs.
        // Keep the proven scalar kernel, specialized at compile time.
        self.chunk_map(libm::log, PAR_ELEMENTS)
    }
    pub fn sqrt(&self) -> Self {
        self.chunk_map(f64::sqrt, PAR_ELEMENTS)
    }
    pub fn abs(&self) -> Self {
        // This memory-bound operation needs more data to amortize scheduling.
        self.chunk_map(f64::abs, PAR_ELEMENTS * 4)
    }
    pub fn floor(&self) -> Self {
        self.chunk_map(f64::floor, PAR_ELEMENTS)
    }
    pub fn ceil(&self) -> Self {
        self.chunk_map(f64::ceil, PAR_ELEMENTS)
    }
    pub fn round(&self) -> Self {
        self.chunk_map(f64::round, PAR_ELEMENTS)
    }

    fn chunk_map(&self, scalar: impl Fn(f64) -> f64 + Send + Sync, parallel_at: usize) -> Self {
        let output = if self.len() >= parallel_at && rayon::current_num_threads() > 1 {
            let mut output = vec![0.; self.len()];
            output
                .par_chunks_mut(CHUNK)
                .zip(self.data().par_chunks(CHUNK))
                .for_each(|(output, input)| {
                    for (output, &x) in output.iter_mut().zip(input) {
                        *output = scalar(x);
                    }
                });
            output
        } else {
            // Exact-size collection initializes each output once, without a zeroing pass.
            self.data().iter().map(|&x| scalar(x)).collect()
        };
        Self::from_column_vec(self.rows(), self.cols(), output).expect("validated input shape")
    }

    fn vector_map<S, V, A>(&self, scalar: S, vector: V, accepts: A) -> Self
    where
        S: Fn(f64) -> f64 + Copy + Send + Sync,
        V: Fn(f64x4) -> f64x4 + Copy + Send + Sync,
        A: Fn(f64) -> bool + Copy + Send + Sync,
    {
        if self.len() < 32 {
            return self.map(scalar);
        }
        let mut output = vec![0.; self.len()];
        if self.len() >= PAR_ELEMENTS {
            output
                .par_chunks_mut(CHUNK)
                .zip(self.data().par_chunks(CHUNK))
                .for_each(|(output, input)| vector_chunk(input, output, scalar, vector, accepts));
        } else {
            vector_chunk(self.data(), &mut output, scalar, vector, accepts);
        }
        Self::from_column_vec(self.rows(), self.cols(), output).expect("validated input shape")
    }
}

fn vector_tan(input: f64x4) -> f64x4 {
    let mut output = input.tan().to_array();
    for (y, x) in output.iter_mut().zip(input.to_array()) {
        // Range-reduction error is magnified near tangent's zeros and poles.
        // Use full scalar reduction in those ill-conditioned neighborhoods.
        if x.abs() > 1. && (y.abs() < 1e-8 || y.abs() > 1e8) {
            *y = libm::tan(x);
        }
    }
    f64x4::from(output)
}

fn vector_chunk<S, V, A>(input: &[f64], output: &mut [f64], scalar: S, vector: V, accepts: A)
where
    S: Fn(f64) -> f64,
    V: Fn(f64x4) -> f64x4,
    A: Fn(f64) -> bool,
{
    let (input_chunks, input_tail) = input.as_chunks::<4>();
    let (output_chunks, output_tail) = output.as_chunks_mut::<4>();
    for (input, output) in input_chunks.iter().zip(output_chunks.iter_mut()) {
        if input.iter().all(|&x| accepts(x)) {
            output.copy_from_slice(&vector(f64x4::from(*input)).to_array());
        } else {
            for (output, &input) in output.iter_mut().zip(input) {
                *output = scalar(input);
            }
        }
    }
    for (output, &input) in output_tail.iter_mut().zip(input_tail) {
        *output = scalar(input);
    }
}
