//! Pure generators for the classic test matrices. All return column-major data
//! for an `n`-by-`n` result unless noted; callers bound `n` first.
use crate::{Result, dsp::check_cancel};
use std::sync::atomic::AtomicBool;

// Matrix definition: Higham, "The Test Matrix Toolbox for MATLAB" (1995),
// magic entry; the public layout fixtures specify orientation.
pub(super) fn magic(n: usize) -> Vec<f64> {
    if n == 0 {
        return Vec::new();
    }
    if n == 2 {
        return vec![1., 4., 3., 2.];
    }
    let mut out = vec![0.; n * n];
    if n % 2 == 1 {
        // De la Loubere's construction: Andrews, Magic Squares and Cubes
        // (1917), ch. I. Start at the top centre; move northeast, or down
        // when the next cell is occupied. Store directly by columns.
        let (mut row, mut col) = (0, n / 2);
        for value in 1..=n * n {
            out[col * n + row] = value as f64;
            let next_row = (row + n - 1) % n;
            let next_col = (col + 1) % n;
            if out[next_col * n + next_row] == 0. {
                (row, col) = (next_row, next_col);
            } else {
                row = (row + 1) % n;
            }
        }
    } else if n.is_multiple_of(4) {
        // Complement the diagonals of each 4-by-4 tile in the rowwise
        // enumeration (Andrews, 1917, ch. I, even-order squares).
        for col in 0..n {
            for row in 0..n {
                let value = row * n + col + 1;
                let complement = row % 4 == col % 4 || row % 4 + col % 4 == 3;
                out[col * n + row] = if complement { n * n + 1 - value } else { value } as f64;
            }
        }
    } else {
        // Strachey's method for singly even orders: four odd-order magic
        // quadrants with offsets [0, 2; 3, 1] times half^2, then swap
        // columns between the upper and lower quadrants (W. W. Rouse Ball,
        // Mathematical Recreations and Essays, 11th ed., 1939, ch. 7). The
        // public fixtures fix the orientation.
        let half = n / 2;
        let quarter = half * half;
        let odd = magic(half);
        for col in 0..half {
            for row in 0..half {
                let value = odd[col * half + row];
                out[col * n + row] = value;
                out[(col + half) * n + row] = value + (2 * quarter) as f64;
                out[col * n + row + half] = value + (3 * quarter) as f64;
                out[(col + half) * n + row + half] = value + quarter as f64;
            }
        }
        let width = half / 2;
        for row in 0..half {
            // Move the middle row's left strip one cell to the right.
            let start = usize::from(row == width);
            for col in start..start + width {
                out.swap(col * n + row, col * n + row + half);
            }
            for col in n - (width - 1)..n {
                out.swap(col * n + row, col * n + row + half);
            }
        }
    }
    out
}

pub(super) fn hilb(n: usize) -> Vec<f64> {
    let mut out = Vec::with_capacity(n * n);
    for j in 0..n {
        out.extend((0..n).map(|i| 1. / (i + j + 1) as f64));
    }
    out
}

pub(super) fn invhilb(n: usize) -> Vec<f64> {
    // Choi, "Tricks or Treats with the Hilbert Matrix" (1983), inverse
    // formula, pp. 301-312. Factoring its binomials gives, in one-based
    // indices, H^-1(i,j) = (-1)^(i+j) w(i) w(j) / (i+j-1), where
    // w(i) = n C(n-1,i-1) C(n+i-1,i-1).
    // Compute each weight independently: a recurrence across weights could
    // overflow near the middle and incorrectly leave finite end weights infinite.
    // While weights and their product fit in u128 the entry is evaluated in
    // exact integers and rounded once; beyond that, in floating point.
    let exact: Vec<Option<u128>> = (0..n)
        .map(|i| {
            (n as u128)
                .checked_mul(binomial_exact(n - 1, i)?)?
                .checked_mul(binomial_exact(n + i, i)?)
        })
        .collect();
    let weights: Vec<_> = (0..n)
        .map(|i| match exact[i] {
            Some(w) => w as f64,
            None => n as f64 * binomial(n - 1, i) * binomial(n + i, i),
        })
        .collect();
    let mut out = vec![0.; n * n];
    for col in 0..n {
        for row in 0..=col {
            let divisor = row + col + 1;
            let magnitude = match (exact[row], exact[col]) {
                (Some(a), Some(b)) if a.checked_mul(b).is_some() => {
                    (a * b / divisor as u128) as f64
                }
                _ => {
                    let product = weights[row] * weights[col];
                    if product.is_finite() {
                        product / divisor as f64
                    } else {
                        // Division first avoids overflowing a finite final entry.
                        (weights[row] / divisor as f64) * weights[col]
                    }
                    .round()
                }
            };
            let value = if (row + col) % 2 == 0 {
                magnitude
            } else {
                -magnitude
            };
            out[col * n + row] = value;
            out[row * n + col] = value;
        }
    }
    out
}

/// C(n, k) in exact integers, or None once it overflows u128.
fn binomial_exact(n: usize, k: usize) -> Option<u128> {
    let k = k.min(n - k);
    let mut value = 1u128;
    for i in 1..=k {
        // Multiply before dividing: C(n-k+i, i) is an integer at every step.
        value = value.checked_mul((n - k + i) as u128)? / i as u128;
    }
    Some(value)
}

fn binomial(n: usize, k: usize) -> f64 {
    if let Some(value) = binomial_exact(n, k) {
        return value as f64;
    }
    // Beyond u128 the value is far past 2^53: rounding is unavoidable.
    let k = k.min(n - k);
    let mut value = 1.;
    for i in 1..=k {
        value *= (n - k + i) as f64 / i as f64;
    }
    value
}

pub(super) fn pascal(n: usize, k: u8, cancel: &AtomicBool) -> Result<Vec<f64>> {
    // Higham, Accuracy and Stability of Numerical Algorithms, 2nd ed.
    // (2002), section 28.4. Pascal's addition identity constructs both
    // P(i,j) = C(i+j,i) and L(i,j) = (-1)^j C(i,j), using zero-based indices.
    check_cancel(cancel)?;
    let mut out = vec![0.; n * n];
    if k == 0 {
        for col in 0..n {
            check_cancel(cancel)?;
            for row in 0..n {
                out[col * n + row] = if row == 0 || col == 0 {
                    1.
                } else {
                    out[col * n + row - 1] + out[(col - 1) * n + row]
                };
            }
        }
    } else {
        // A single unsigned Pascal row also supplies the permuted transpose
        // directly. For J reversing columns, Q = (-1)^(n-1) L^T J has Q^3=I;
        // the parity factor is needed for even n.
        let mut coefficients = vec![1.; n];
        for row in 0..n {
            check_cancel(cancel)?;
            for (col, &coefficient) in coefficients.iter().enumerate().take(row + 1) {
                let (index, negative) = if k == 1 {
                    (col * n + row, col % 2 == 1)
                } else {
                    ((n - 1 - row) * n + col, (n - 1 + col) % 2 == 1)
                };
                out[index] = if negative { -coefficient } else { coefficient };
            }
            for col in (1..=row).rev() {
                coefficients[col] += coefficients[col - 1];
            }
        }
    }
    check_cancel(cancel)?;
    Ok(out)
}

pub(super) fn rosser() -> Vec<f64> {
    const R: [[f64; 8]; 8] = [
        [611., 196., -192., 407., -8., -52., -49., 29.],
        [196., 899., 113., -192., -71., -43., -8., -44.],
        [-192., 113., 899., 196., 61., 49., 8., 52.],
        [407., -192., 196., 611., 8., 44., 59., -23.],
        [-8., -71., 61., 8., 411., -599., 208., 208.],
        [-52., -43., 49., 44., -599., 411., 208., 208.],
        [-49., -8., 8., 59., 208., 208., 99., -911.],
        [29., -44., 52., -23., 208., 208., -911., 99.],
    ];
    (0..8).flat_map(|j| (0..8).map(move |i| R[i][j])).collect()
}

/// Symmetric tridiagonal W(n)+ of Wilkinson: |(-(n-1)/2:(n-1)/2)| on the
/// diagonal, ones beside it.
pub(super) fn wilkinson(n: usize) -> Vec<f64> {
    let mut out = vec![0.; n * n];
    for i in 0..n {
        out[i * n + i] = ((i as f64) - (n as f64 - 1.) / 2.).abs();
        if i + 1 < n {
            out[i * n + i + 1] = 1.;
            out[(i + 1) * n + i] = 1.;
        }
    }
    out
}

/// Companion matrix (column-major, `(n-1)`-square) of coefficients `u`
/// (`n = u.len()`); first row is `-u(2:n)/u(1)`, ones on the subdiagonal.
pub(super) fn compan(u: &[f64]) -> Vec<f64> {
    let m = u.len() - 1;
    let mut out = vec![0.; m * m];
    for (j, &c) in u[1..].iter().enumerate() {
        out[j * m] = -c / u[0];
    }
    for i in 1..m {
        out[(i - 1) * m + i] = 1.;
    }
    out
}

/// `n = 2^e * base` with base in {1, 12, 20}, as the reference's `hadamard` accepts.
fn hadamard_split(n: usize) -> Option<(u32, usize)> {
    [1usize, 12, 20].into_iter().find_map(|p| {
        (n.is_multiple_of(p) && (n / p).is_power_of_two() && n > 0)
            .then(|| ((n / p).trailing_zeros(), p))
    })
}

pub(super) fn hadamard_order(n: usize) -> bool {
    hadamard_split(n).is_some()
}

/// First row of the 19-by-19 Hankel block of the reference's `hadamard(20)`, followed
/// by the rest of the 37-term sequence (read off the reference output).
const HADAMARD_20: [i8; 37] = [
    -1, -1, 1, 1, -1, -1, -1, -1, 1, -1, 1, -1, 1, 1, 1, 1, -1, -1, 1, -1, -1, 1, 1, -1, -1, -1,
    -1, 1, -1, 1, -1, 1, 1, 1, 1, -1, -1,
];

/// Hadamard matrix of order `n = 2^e * p`, p in {1, 12, 20}, as the reference's
/// `hadamard` (n, n/12 or n/20 a power of 2); row-major +-1 data. Sylvester
/// doubling as in MATLAB; the 12 block is Paley I, the 20 block the Hankel
/// matrix of `HADAMARD_20`.
pub(super) fn hadamard(n: usize) -> Option<Vec<f64>> {
    let (e, base) = hadamard_split(n)?;
    let mut size = base;
    let mut h = match base {
        1 => vec![1.],
        12 => paley_one(11),
        _ => {
            let mut h = vec![1.; 400];
            for i in 1..20 {
                for j in 1..20 {
                    h[i * 20 + j] = f64::from(HADAMARD_20[i + j - 2]);
                }
            }
            h
        }
    };
    for _ in 0..e {
        let mut next = vec![0.; 4 * size * size];
        for i in 0..size {
            for j in 0..size {
                let v = h[i * size + j];
                next[i * 2 * size + j] = v;
                next[i * 2 * size + size + j] = v;
                next[(i + size) * 2 * size + j] = v;
                next[(i + size) * 2 * size + size + j] = -v;
            }
        }
        h = next;
        size *= 2;
    }
    Some(h)
}

fn character(q: usize) -> Vec<f64> {
    let mut chi = vec![-1.; q];
    chi[0] = 0.;
    for x in 1..q {
        chi[x * x % q] = 1.;
    }
    chi
}

/// Paley I (q = 3 mod 4 prime), order q+1, first row and column all ones.
fn paley_one(q: usize) -> Vec<f64> {
    let chi = character(q);
    let n = q + 1;
    let mut h = vec![1.; n * n];
    for i in 1..n {
        for j in 1..n {
            let diagonal = if i == j { 1. } else { 0. };
            h[i * n + j] = -(chi[(i + q - j) % q] + diagonal);
        }
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magic_layouts_and_permutations() {
        assert!(magic(0).is_empty());
        assert_eq!(magic(2), [1., 4., 3., 2.]);
        assert_eq!(magic(3), [8., 3., 4., 1., 5., 9., 6., 7., 2.]);
        assert_eq!(
            magic(4),
            [
                16., 5., 9., 4., 2., 11., 7., 14., 3., 10., 6., 15., 13., 8., 12., 1.
            ]
        );
        assert_eq!(
            magic(6),
            [
                35., 3., 31., 8., 30., 4., 1., 32., 9., 28., 5., 36., 6., 7., 2., 33., 34., 29.,
                26., 21., 22., 17., 12., 13., 19., 23., 27., 10., 14., 18., 24., 25., 20., 15.,
                16., 11.
            ]
        );
        for n in 1..=65 {
            let mut values = magic(n);
            values.sort_by(f64::total_cmp);
            assert_eq!(values, (1..=n * n).map(|v| v as f64).collect::<Vec<_>>());
        }
    }

    #[test]
    fn inverse_hilbert_matches_integer_formula() {
        // Independent small-integer evaluation of Choi's original formula.
        let choose =
            |n: usize, k: usize| (1..=k).fold(1u128, |v, t| v * (n - k + t) as u128 / t as u128);
        assert!(invhilb(0).is_empty());
        for n in 1..=12 {
            let inverse = invhilb(n);
            for i in 1..=n {
                for j in 1..=n {
                    let magnitude = (i + j - 1) as u128
                        * choose(n + i - 1, n - j)
                        * choose(n + j - 1, n - i)
                        * choose(i + j - 2, i - 1).pow(2);
                    let expected = magnitude as f64 * if (i + j) % 2 == 0 { 1. } else { -1. };
                    let actual = inverse[(j - 1) * n + i - 1];
                    assert_eq!(actual, expected, "n={n}, i={i}, j={j}");
                }
            }
        }
    }

    #[test]
    fn binomial_and_inverse_hilbert_are_exact_where_representable() {
        // C(55,27) = 3824345300380220 < 2^53; a divide-first float recurrence misses it.
        assert_eq!(binomial(55, 27), 3_824_345_300_380_220.);
        // Exactly representable entry that a rounded-weight product got wrong.
        let n = 26;
        assert_eq!(invhilb(n)[n - 1], -6_446_940_928_325_352.);
    }

    #[test]
    fn inverse_hilbert_overflow_preserves_finite_edges() {
        let n = 450;
        let inverse = invhilb(n);
        assert_eq!(inverse[0], (n * n) as f64);
        assert!(inverse[n - 1].is_finite());
        assert!(inverse[n - 1] < 0.);
        assert!(inverse[(n - 1) * n + n - 1].is_infinite());
        for col in 0..n {
            for row in 0..n {
                let value = inverse[col * n + row];
                assert!(!value.is_nan());
                assert_eq!(value, inverse[row * n + col]);
                assert_eq!(value.is_sign_negative(), (row + col) % 2 == 1);
            }
        }
    }

    fn multiply(a: &[f64], b: &[f64], n: usize) -> Vec<f64> {
        (0..n * n)
            .map(|index| {
                let (row, col) = (index % n, index / n);
                (0..n).map(|k| a[k * n + row] * b[col * n + k]).sum()
            })
            .collect()
    }

    #[test]
    fn pascal_factors_have_the_prescribed_powers() {
        let idle = AtomicBool::new(false);
        assert_eq!(
            pascal(3, 1, &idle).unwrap(),
            [1., 1., 1., 0., -1., -2., 0., 0., 1.]
        );
        assert_eq!(
            pascal(3, 2, &idle).unwrap(),
            [1., -2., 1., 1., -1., 0., 1., 0., 0.]
        );
        for n in 0..=10 {
            let l = pascal(n, 1, &idle).unwrap();
            let q = pascal(n, 2, &idle).unwrap();
            let l_squared = multiply(&l, &l, n);
            let q_cubed = multiply(&multiply(&q, &q, n), &q, n);
            for col in 0..n {
                for row in 0..n {
                    let expected = if row == col { 1. } else { 0. };
                    assert_eq!(l_squared[col * n + row], expected, "L^2 n={n}");
                    assert_eq!(q_cubed[col * n + row], expected, "Q^3 n={n}");
                }
            }
        }
    }

    #[test]
    fn pascal_large_values_and_empty_cancellation() {
        let idle = AtomicBool::new(false);
        assert_eq!(pascal(30, 0, &idle).unwrap()[899], 30_067_266_499_541_040.);
        let n = 1500;
        let p = pascal(n, 0, &idle).unwrap();
        assert_eq!(p[n - 1], 1.);
        assert_eq!(p[(n - 1) * n], 1.);
        assert_eq!(p[(n - 1) * n + 1], 1500.);
        assert_eq!(p[3 * n + 2], 10.);
        assert!(p[n * n - 1].is_infinite());
        assert!(p.iter().all(|v| !v.is_nan()));
        let stop = AtomicBool::new(true);
        for k in 0..3 {
            assert_eq!(pascal(0, k, &stop).unwrap_err(), "Calculation cancelled");
        }
    }

    #[test]
    fn magic_squares_have_constant_sums() {
        for n in [1usize, 3, 4, 5, 6, 7, 8, 9, 10, 12, 14, 18, 30] {
            let m = magic(n);
            let target = (n * (n * n + 1) / 2) as f64;
            let at = |i: usize, j: usize| m[j * n + i];
            for i in 0..n {
                assert_eq!((0..n).map(|j| at(i, j)).sum::<f64>(), target, "row n={n}");
                assert_eq!((0..n).map(|j| at(j, i)).sum::<f64>(), target, "col n={n}");
            }
            assert_eq!((0..n).map(|i| at(i, i)).sum::<f64>(), target, "diag n={n}");
            assert_eq!(
                (0..n).map(|i| at(i, n - 1 - i)).sum::<f64>(),
                target,
                "anti n={n}"
            );
        }
    }

    #[test]
    fn hadamard_rows_are_orthogonal() {
        for n in [1usize, 2, 4, 8, 12, 20, 24, 40, 48] {
            let h = hadamard(n).unwrap();
            for a in 0..n {
                for b in 0..n {
                    let dot: f64 = (0..n).map(|k| h[a * n + k] * h[b * n + k]).sum();
                    assert_eq!(dot, if a == b { n as f64 } else { 0. }, "n={n}");
                }
            }
        }
        assert!(
            hadamard(3).is_none()
                && hadamard(0).is_none()
                && hadamard(36).is_none()
                && hadamard(28).is_none()
        );
    }

    #[test]
    fn pascal_default_is_the_factor_product_and_cancels() {
        let idle = AtomicBool::new(false);
        for n in [0usize, 1, 2, 5, 9] {
            let l = pascal(n, 1, &idle).unwrap();
            let p = pascal(n, 0, &idle).unwrap();
            for i in 0..n {
                for j in 0..n {
                    let product: f64 = (0..n).map(|t| l[t * n + i] * l[t * n + j]).sum();
                    assert_eq!(p[j * n + i], product, "n={n} ({i},{j})");
                }
            }
        }
        let stop = AtomicBool::new(true);
        for k in 0..3 {
            assert_eq!(pascal(4, k, &stop).unwrap_err(), "Calculation cancelled");
        }
    }
}
