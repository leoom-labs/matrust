//! Data exchanged by the scientific-format readers (HDF5 h5read, NetCDF
//! ncread): values, MATLAB-order dimensions and hyperslab selections.
use crate::{NumericClass, Result};

/// Values of a dataset or variable, in column-major MATLAB order.
#[derive(Debug, Clone)]
pub enum H5Data {
    Double(Vec<f64>),
    Single(Vec<f32>),
    Int(NumericClass, Vec<i128>),
    Logical(Vec<u8>),
    Text(Vec<String>),
    /// A character array (NetCDF NC_CHAR).
    Chars(Vec<u16>),
}

/// Data with MATLAB-order dimensions (at least two).
#[derive(Debug, Clone)]
pub struct H5Array {
    pub dims: Vec<usize>,
    pub data: H5Data,
}

/// A hyperslab in MATLAB order: zero-based start, count (None: to the end)
/// and stride per dimension.
#[derive(Debug, Clone)]
pub struct H5Selection {
    pub start: Vec<usize>,
    pub count: Vec<Option<usize>>,
    pub stride: Vec<usize>,
}

/// Numeric attributes ncread applies.
#[derive(Debug, Clone, Default)]
pub struct NcAttributes {
    pub fill: Option<f64>,
    pub missing: Option<f64>,
    pub scale: Option<f64>,
    pub offset: Option<f64>,
}

/// One selected dimension in C order: start, count, step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Span {
    pub start: usize,
    pub count: usize,
    pub step: usize,
}

/// MATLAB-order dimensions of a C-order shape.
pub fn matlab_dims(shape: &[usize]) -> Vec<usize> {
    match shape {
        [] => vec![1, 1],
        [n] => vec![*n, 1],
        _ => shape.iter().rev().copied().collect(),
    }
}

/// The C-order spans `sel` (MATLAB order) picks from a C-order `shape`.
/// A one-dimensional shape also takes two-element MATLAB vectors whose
/// second element is the trivial one (start 0, count 1, stride 1).
pub fn resolve_selection(shape: &[usize], sel: &H5Selection) -> Result<Vec<Span>> {
    if shape.is_empty() {
        return Err("a scalar cannot be read in parts; read all of it".into());
    }
    let rank = shape.len();
    let fit = |len: usize, what: &str, trivial: bool| -> Result<()> {
        if len == rank || rank == 1 && len == 2 && trivial {
            Ok(())
        } else {
            Err(format!(
                "{what} must have {rank} element(s), one per dimension"
            ))
        }
    };
    fit(sel.start.len(), "start", sel.start.get(1) == Some(&0))?;
    fit(sel.count.len(), "count", sel.count.get(1) == Some(&Some(1)))?;
    fit(sel.stride.len(), "stride", sel.stride.get(1) == Some(&1))?;
    let mut spans = Vec::with_capacity(rank);
    for (k, &dim) in shape.iter().enumerate() {
        let m = rank - 1 - k;
        let (s, step) = (sel.start[m], sel.stride[m]);
        if step == 0 {
            return Err("stride values must be 1 or greater".into());
        }
        if s >= dim.max(1) {
            return Err(format!(
                "start {} is past the end of a dimension of length {dim}",
                s + 1
            ));
        }
        let count = match sel.count[m] {
            Some(c) => c,
            None if dim == 0 => 0,
            None => (dim - s - 1) / step + 1,
        };
        if count > 0
            && (count - 1)
                .checked_mul(step)
                .and_then(|x| x.checked_add(s))
                .is_none_or(|last| last >= dim)
        {
            return Err(format!(
                "start {}, count {count} and stride {step} reach past a dimension of length {dim}",
                s + 1
            ));
        }
        spans.push(Span {
            start: s,
            count,
            step,
        });
    }
    Ok(spans)
}

/// Element count of C-order `counts`, bounded by `max_bytes` at `size` bytes each.
pub fn checked_elements(counts: &[usize], size: usize, max_bytes: usize) -> Result<usize> {
    let n = counts
        .iter()
        .try_fold(1usize, |n, &d| n.checked_mul(d))
        .ok_or("the selection is too large")?;
    if n.saturating_mul(size.max(1)) > max_bytes {
        return Err(format!(
            "the data exceeds the {} MiB read limit",
            max_bytes >> 20
        ));
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sel(start: &[usize], count: &[Option<usize>], stride: &[usize]) -> H5Selection {
        H5Selection {
            start: start.to_vec(),
            count: count.to_vec(),
            stride: stride.to_vec(),
        }
    }

    #[test]
    fn matlab_order_reverses_c_order() {
        // C shape (3, 4) is 4x3 in MATLAB: MATLAB start (2, 1) is C (1, 2).
        let spans = resolve_selection(&[3, 4], &sel(&[1, 0], &[Some(2), None], &[1, 1])).unwrap();
        assert_eq!(
            spans[0],
            Span {
                start: 0,
                count: 3,
                step: 1
            }
        );
        assert_eq!(
            spans[1],
            Span {
                start: 1,
                count: 2,
                step: 1
            }
        );
        let spans = resolve_selection(&[10], &sel(&[1, 0], &[None, Some(1)], &[3, 1])).unwrap();
        assert_eq!(
            spans,
            [Span {
                start: 1,
                count: 3,
                step: 3
            }]
        );
        assert!(resolve_selection(&[10], &sel(&[0, 0], &[Some(11), Some(1)], &[1, 1])).is_err());
        assert!(resolve_selection(&[3, 4], &sel(&[0], &[None], &[1])).is_err());
    }
}
