//! h5read, h5create, h5write (HDF5 through the mat-v73 libhdf5) and ncread
//! (NetCDF classic in pure Rust, NetCDF-4 through libhdf5). Behaviour
//! follows the published MathWorks function pages (behaviour only);
//! differences are listed on the help pages.
use super::{rdx_pairs, rdx_text};
use crate::{
    Array, CellArray, CharacterRow, LogicalMatrix, Matrix, NumericClass, Result, TypedArray,
    engine::{Engine, Value},
    files::array_data::{H5Array, H5Data, H5Selection, NcAttributes},
};
use std::sync::Arc;

#[cfg(not(feature = "mat-v73"))]
const NO_HDF5: &str =
    "HDF5 files need the desktop or command-line MatRust (not available in the browser build)";

/// Real values of a numeric vector argument.
fn h5_reals(function: &str, what: &str, value: &Value) -> Result<Vec<f64>> {
    let bad = || format!("{function}: {what} must be a vector of numbers");
    let v = match value {
        Value::Matrix(m) => m.data().to_vec(),
        Value::Container(Array::Typed(t)) => t.to_f64_vec()?,
        _ => return Err(bad()),
    };
    if v.is_empty() || v.len() > 32 || v.iter().any(|x| x.is_nan()) {
        return Err(bad());
    }
    Ok(v)
}

/// One-based positive integers (start, stride) as zero-based or plain values.
fn h5_indices(function: &str, what: &str, value: &Value, one_based: bool) -> Result<Vec<usize>> {
    h5_reals(function, what, value)?
        .into_iter()
        .map(|x| {
            if x >= 1. && x.fract() == 0. && x <= 1e15 {
                Ok(x as usize - usize::from(one_based))
            } else {
                Err(format!(
                    "{function}: {what} values must be positive integers"
                ))
            }
        })
        .collect()
}

/// count: nonnegative integers, Inf meaning "to the end".
fn h5_counts(function: &str, value: &Value) -> Result<Vec<Option<usize>>> {
    h5_reals(function, "count", value)?
        .into_iter()
        .map(|x| {
            if x == f64::INFINITY {
                Ok(None)
            } else if x >= 0. && x.fract() == 0. && x <= 1e15 {
                Ok(Some(x as usize))
            } else {
                Err(format!(
                    "{function}: count values must be nonnegative integers or Inf"
                ))
            }
        })
        .collect()
}

/// Optional start, count, stride arguments.
fn h5_selection(function: &str, rest: &[Value]) -> Result<Option<H5Selection>> {
    match rest {
        [] => Ok(None),
        [_] => Err(format!("{function}: give both start and count")),
        [start, count, stride @ ..] if stride.len() <= 1 => {
            let start = h5_indices(function, "start", start, true)?;
            let count = h5_counts(function, count)?;
            let stride = match stride.first() {
                Some(s) => h5_indices(function, "stride", s, false)?,
                None => vec![1; start.len()],
            };
            if count.len() != start.len() || stride.len() != start.len() {
                return Err(format!(
                    "{function}: start, count and stride must have the same length"
                ));
            }
            Ok(Some(H5Selection {
                start,
                count,
                stride,
            }))
        }
        _ => Err("Too many input arguments.".into()),
    }
}

fn h5_value(array: H5Array, cancel: &std::sync::atomic::AtomicBool) -> Result<Value> {
    let dims = array.dims;
    // Folded storage: dims[0] rows by the product of the other dimensions.
    let (n, c) = (dims[0], dims[1..].iter().product::<usize>());
    let array = match array.data {
        H5Data::Double(v) => Array::Numeric(Matrix::from_column_vec(n, c, v)?.with_dims(&dims)?),
        H5Data::Single(v) => Array::Typed(
            TypedArray::from_reals(NumericClass::Single, n, c, v.into_iter().map(f64::from))?
                .with_dims(&dims)?,
        ),
        H5Data::Int(class, v) => {
            Array::Typed(TypedArray::from_ints(class, n, c, v.into_iter())?.with_dims(&dims)?)
        }
        H5Data::Logical(v) => {
            Array::Logical(LogicalMatrix::from_column_vec(n, c, v, cancel)?.with_dims(&dims)?)
        }
        H5Data::Chars(v) => {
            Array::Character(CharacterRow::from_column_major(n, c, v, cancel)?.with_dims(&dims)?)
        }
        H5Data::Text(texts) => {
            if dims.len() > 2 {
                return Err("text datasets with more than two dimensions are not supported".into());
            }
            let mut slots = Vec::with_capacity(texts.len());
            for t in &texts {
                slots.push(Arc::new(Array::from(CharacterRow::from_text(t, cancel)?)));
            }
            Array::Cell(CellArray::from_shared(dims[0], dims[1], slots, cancel)?)
        }
    };
    Ok(Value::from(array))
}

/// ncread: fill and missing values become NaN in floating data; with
/// scale_factor or add_offset the result is double, unpacked.
fn nc_apply(array: H5Array, attrs: &NcAttributes) -> H5Array {
    let packed = attrs.scale.is_some() || attrs.offset.is_some();
    let is_gap = |x: f64| attrs.fill == Some(x) || attrs.missing == Some(x);
    let data = match array.data {
        H5Data::Double(v) if !packed => H5Data::Double(
            v.into_iter()
                .map(|x| if is_gap(x) { f64::NAN } else { x })
                .collect(),
        ),
        H5Data::Single(v) if !packed => H5Data::Single(
            v.into_iter()
                .map(|x| if is_gap(f64::from(x)) { f32::NAN } else { x })
                .collect(),
        ),
        data if packed => {
            let reals: Vec<f64> = match data {
                H5Data::Double(v) => v,
                H5Data::Single(v) => v.into_iter().map(f64::from).collect(),
                H5Data::Int(_, v) => v.into_iter().map(|x| x as f64).collect(),
                other => {
                    return H5Array {
                        dims: array.dims,
                        data: other,
                    };
                }
            };
            let (scale, offset) = (attrs.scale.unwrap_or(1.), attrs.offset.unwrap_or(0.));
            H5Data::Double(
                reals
                    .into_iter()
                    .map(|x| {
                        if is_gap(x) {
                            f64::NAN
                        } else {
                            x * scale + offset
                        }
                    })
                    .collect(),
            )
        }
        other => other,
    };
    H5Array {
        dims: array.dims,
        data,
    }
}

/// The values and MATLAB dimensions of h5write data.
#[cfg(feature = "mat-v73")]
fn h5_data(value: &Value, cancel: &std::sync::atomic::AtomicBool) -> Result<(H5Data, Vec<usize>)> {
    let array = value.clone().array(cancel)?;
    let dims = array.dims().into_owned();
    let data = match &array {
        Array::Numeric(m) => H5Data::Double(m.data().to_vec()),
        Array::Logical(m) => H5Data::Logical(m.data().to_vec()),
        Array::Typed(t) if t.class() == NumericClass::Single => {
            H5Data::Single(t.to_f64_vec()?.into_iter().map(|x| x as f32).collect())
        }
        Array::Typed(t) => H5Data::Int(t.class(), (0..t.len()).map(|i| t.int(i)).collect()),
        Array::Complex(_) => return Err("h5write: complex data is not supported".into()),
        _ => return Err("h5write: the data must be numeric or logical".into()),
    };
    Ok((data, dims))
}

impl Engine {
    fn h5_file(
        &self,
        function: &str,
        value: Option<&Value>,
        must_exist: bool,
    ) -> Result<std::path::PathBuf> {
        let name = value
            .and_then(rdx_text)
            .ok_or_else(|| format!("{function}: the file name must be text"))?;
        let path = self.file_path(&name, "")?;
        if must_exist && !path.is_file() {
            return Err(format!("{function}: unable to find file {name}"));
        }
        Ok(path)
    }

    fn h5_name(function: &str, value: Option<&Value>, what: &str) -> Result<String> {
        value
            .and_then(rdx_text)
            .ok_or_else(|| format!("{function}: the {what} name must be text"))
    }

    pub(super) fn h5_read(&mut self, args: &[Value]) -> Result<Value> {
        let path = self.h5_file("h5read", args.first(), true)?;
        let name = Self::h5_name("h5read", args.get(1), "dataset")?;
        let sel = h5_selection("h5read", args.get(2..).unwrap_or(&[]))?;
        #[cfg(feature = "mat-v73")]
        {
            let array = crate::files::hdf5_data::h5_read(&path, &name, sel.as_ref(), &self.cancel)
                .map_err(|e| format!("h5read: {e}"))?;
            h5_value(array, &self.cancel)
        }
        #[cfg(not(feature = "mat-v73"))]
        {
            let _ = (path, name, sel);
            Err(NO_HDF5.into())
        }
    }

    pub(super) fn h5_create(&mut self, args: &[Value]) -> Result<()> {
        let path = self.h5_file("h5create", args.first(), false)?;
        let name = Self::h5_name("h5create", args.get(1), "dataset")?;
        let size = args
            .get(2)
            .ok_or("h5create: give the dataset size, as in h5create(file, '/data', [10 20])")?;
        let size = h5_reals("h5create", "the size", size)?;
        if size.contains(&f64::INFINITY) {
            return Err("h5create: unlimited (Inf) dimensions are not supported yet".into());
        }
        let dims: Vec<usize> = size
            .iter()
            .map(|&x| {
                if x >= 0. && x.fract() == 0. && x <= 1e12 {
                    Ok(x as usize)
                } else {
                    Err("h5create: size values must be nonnegative integers".to_owned())
                }
            })
            .collect::<Result<_>>()?;
        let mut class = "double".to_owned();
        let mut chunk = None;
        let mut deflate = None;
        let mut fill = None;
        for (option, value) in rdx_pairs("h5create", args.get(3..).unwrap_or(&[]))? {
            match option.as_str() {
                "datatype" => {
                    class = rdx_text(value)
                        .ok_or("h5create: Datatype must be text")?
                        .to_ascii_lowercase();
                }
                "chunksize" => chunk = Some(h5_indices("h5create", "ChunkSize", value, false)?),
                "deflate" => {
                    let level = h5_reals("h5create", "Deflate", value)?;
                    match level.as_slice() {
                        [l] if (0. ..=9.).contains(l) && l.fract() == 0. => {
                            deflate = Some(*l as u8)
                        }
                        _ => return Err("h5create: Deflate must be an integer from 0 to 9".into()),
                    }
                }
                "fillvalue" => match h5_reals("h5create", "FillValue", value)?.as_slice() {
                    [v] => fill = Some(*v),
                    _ => return Err("h5create: FillValue must be a scalar".into()),
                },
                _ => {
                    return Err(format!(
                        "h5create: unsupported option '{option}'; supported are Datatype, ChunkSize, Deflate and FillValue"
                    ));
                }
            }
        }
        #[cfg(feature = "mat-v73")]
        {
            crate::files::hdf5_data::h5_create(
                &path,
                &name,
                &dims,
                &class,
                chunk.as_deref(),
                deflate,
                fill,
            )
            .map_err(|e| format!("h5create: {e}"))
        }
        #[cfg(not(feature = "mat-v73"))]
        {
            let _ = (path, name, dims, class, chunk, deflate, fill);
            Err(NO_HDF5.into())
        }
    }

    pub(super) fn h5_write(&mut self, args: &[Value]) -> Result<()> {
        let path = self.h5_file("h5write", args.first(), true)?;
        let name = Self::h5_name("h5write", args.get(1), "dataset")?;
        let data = args.get(2).ok_or("h5write: give the data to write")?;
        let sel = h5_selection("h5write", args.get(3..).unwrap_or(&[]))?;
        #[cfg(feature = "mat-v73")]
        {
            let (values, dims) = h5_data(data, &self.cancel)?;
            let (start, stride) = match &sel {
                Some(sel) => {
                    // count must be the size of the data.
                    let mut want: Vec<usize> = dims.clone();
                    let given: Vec<usize> = sel
                        .count
                        .iter()
                        .map(|c| c.ok_or("h5write: count cannot be Inf"))
                        .collect::<std::result::Result<_, _>>()?;
                    want.resize(want.len().max(given.len()), 1);
                    let mut padded = given.clone();
                    padded.resize(want.len(), 1);
                    // A single count value describes a vector of that length.
                    let numel: usize = dims.iter().product();
                    let vector = given.len() == 1 && given[0] == numel;
                    if padded != want && !vector {
                        return Err("h5write: count must match the size of the data".into());
                    }
                    (Some(sel.start.clone()), Some(sel.stride.clone()))
                }
                None => (None, None),
            };
            crate::files::hdf5_data::h5_write(
                &path,
                &name,
                &values,
                &dims,
                start.as_deref(),
                stride.as_deref(),
            )
            .map_err(|e| format!("h5write: {e}"))
        }
        #[cfg(not(feature = "mat-v73"))]
        {
            let _ = (path, name, data, sel);
            Err(NO_HDF5.into())
        }
    }

    pub(super) fn nc_read(&mut self, args: &[Value]) -> Result<Value> {
        let path = self.h5_file("ncread", args.first(), true)?;
        let name = Self::h5_name("ncread", args.get(1), "variable")?;
        let sel = h5_selection("ncread", args.get(2..).unwrap_or(&[]))?;
        let mut head = [0u8; 8];
        let read = std::fs::File::open(&path)
            .and_then(|mut f| std::io::Read::read(&mut f, &mut head))
            .map_err(|e| format!("ncread: {e}"))?;
        let head = &head[..read];
        let (array, attrs) = if crate::files::netcdf3::is_classic(head) {
            crate::files::netcdf3::nc3_read(&path, &name, sel.as_ref(), &self.cancel)
        } else if head.starts_with(b"\x89HDF\r\n\x1a\n") {
            #[cfg(feature = "mat-v73")]
            {
                crate::files::hdf5_data::nc4_read(&path, &name, sel.as_ref(), &self.cancel)
            }
            #[cfg(not(feature = "mat-v73"))]
            {
                Err(NO_HDF5.to_owned())
            }
        } else {
            Err("not a NetCDF file (classic or NetCDF-4)".to_owned())
        }
        .map_err(|e| format!("ncread: {e}"))?;
        h5_value(nc_apply(array, &attrs), &self.cancel)
    }
}
