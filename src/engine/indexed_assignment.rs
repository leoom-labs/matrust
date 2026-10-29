//! Transactional typed indexed assignment, growth and literal [] deletion.
use super::{Engine, Execution, Value, indexing::Indices, stored_array::StoredArray};
use crate::LogicalMatrix;
use crate::{Matrix, Result, matrix::MAX_ELEMENTS, syntax::Expr};
use rayon::prelude::*;

const CHUNK: usize = 8192;

fn covers(indices: &Indices, bound: usize) -> bool {
    if indices.len() != bound {
        return false;
    }
    bound == 0
        || indices.contiguous() == Some(0)
        || matches!(indices, Indices::Stride {start,step:-1,..} if *start==bound-1)
}

fn all(len: usize) -> Indices {
    Indices::Stride {
        start: 0,
        step: 1,
        len,
    }
}

impl Engine {
    fn replacement_buffer<T: StoredArray>(
        &self,
        len: usize,
        value: impl Fn(usize) -> T::Element + Sync,
    ) -> Result<Vec<T::Element>> {
        if len >= T::PAR_COPY && rayon::current_num_threads() > 1 {
            let mut output = self.stored_buffer::<T>(len, true)?;
            output.par_chunks_mut(CHUNK).enumerate().try_for_each(
                |(block, chunk)| -> Result<()> {
                    self.check_cancel()?;
                    for (i, out) in chunk.iter_mut().enumerate() {
                        *out = value(block * CHUNK + i);
                    }
                    Ok(())
                },
            )?;
            Ok(output)
        } else {
            let mut output = self.stored_buffer::<T>(len, false)?;
            for first in (0..len).step_by(CHUNK) {
                self.check_cancel()?;
                output.extend((first..(first + CHUNK).min(len)).map(&value));
            }
            Ok(output)
        }
    }

    fn copy_for_write<T: StoredArray>(
        &self,
        name: &str,
        shape: (usize, usize),
        len: usize,
    ) -> Result<Vec<T::Element>> {
        let old = T::lookup(self, name);
        let (nr, nc) = old.map_or((0, 0), |m| (m.rows(), m.cols()));
        if len >= T::PAR_COPY && rayon::current_num_threads() > 1 {
            let mut data = self.stored_buffer::<T>(len, true)?;
            data.par_chunks_mut(CHUNK).enumerate().try_for_each(
                |(block, chunk)| -> Result<()> {
                    self.check_cancel()?;
                    if let Some(old) = old {
                        let mut first = block * CHUNK;
                        let mut output = chunk;
                        while !output.is_empty() {
                            let (r, c) = (first % shape.0, first / shape.0);
                            let n = output.len().min(shape.0 - r);
                            let (head, tail) = output.split_at_mut(n);
                            if c < nc && r < nr {
                                let copied = n.min(nr - r);
                                head[..copied]
                                    .copy_from_slice(&old.data()[c * nr + r..c * nr + r + copied]);
                            }
                            output = tail;
                            first += n;
                        }
                    }
                    Ok(())
                },
            )?;
            Ok(data)
        } else {
            let mut data = self.stored_buffer::<T>(len, false)?;
            for c in 0..shape.1 {
                if c < nc {
                    for chunk in
                        old.expect("existing columns").data()[c * nr..(c + 1) * nr].chunks(CHUNK)
                    {
                        self.check_cancel()?;
                        data.extend_from_slice(chunk);
                    }
                }
                let end = (c + 1) * shape.0;
                while data.len() < end {
                    self.check_cancel()?;
                    data.resize((data.len() + CHUNK).min(end), T::Element::default());
                }
            }
            Ok(data)
        }
    }

    fn write_contiguous<T: StoredArray>(
        &self,
        destination: &mut [T::Element],
        values: &T,
        first_value: usize,
    ) -> Result<()> {
        for (block, chunk) in destination.chunks_mut(CHUNK).enumerate() {
            self.check_cancel()?;
            if values.len() == 1 {
                chunk.fill(values.data()[0]);
            } else {
                let first = first_value + block * CHUNK;
                chunk.copy_from_slice(&values.data()[first..first + chunk.len()]);
            }
        }
        Ok(())
    }

    fn index_extent(&self, indices: &Indices) -> Result<usize> {
        if indices.len() == 0 {
            return Ok(0);
        }
        match indices {
            Indices::Stride { len, .. } => Ok(indices.at(0).max(indices.at(len - 1)) + 1),
            Indices::LogicalStride { start, len, .. } => Ok(start + len),
            Indices::Mask(values, _) => Ok(values.last().map_or(0, |i| *i as usize + 1)),
            Indices::Explicit(values) => {
                let mut maximum = 0;
                for chunk in values.data().chunks(CHUNK) {
                    self.check_cancel()?;
                    for &value in chunk {
                        maximum = maximum.max(value as usize);
                    }
                }
                Ok(maximum)
            }
        }
    }

    fn write_budget<T: StoredArray>(&mut self, name: &str, shape: (usize, usize)) -> Result<usize> {
        let len = T::checked_len(shape.0, shape.1)?;
        let old = self.array(name).map_or(0, |a| a.retained_bytes());
        if self.workspace_bytes() - old + len * T::BYTES > 256 * 1024 * 1024 {
            let logical = self.obj_workspace_bytes()
                + self
                    .workspace()
                    .values()
                    .map(crate::Array::bytes)
                    .sum::<usize>();
            let old_logical = self.array(name).map_or(0, |a| a.bytes());
            if logical - old_logical + len * T::BYTES > 256 * 1024 * 1024 {
                return Err(
                    "Workspace exceeds its 256 MiB logical storage limit; use clear".into(),
                );
            }
            // Final assign_workspace_array performs the cancellable fact-only
            // fallback atomically once the dense result has been constructed.
        }
        self.charge(len * T::BYTES.div_ceil(8))?;
        self.check_cancel()?;
        Ok(len)
    }
    fn assign_array<T: StoredArray>(
        &mut self,
        name: &str,
        result: T,
        quiet: bool,
        out: &mut Execution,
    ) -> Result<()> {
        self.assign_workspace_array(name, result.value().array(&self.cancel)?, quiet, out)
    }
    pub(super) fn assign_indexed(
        &mut self,
        name: &str,
        args: &[Expr],
        rhs: &Expr,
        quiet: bool,
        out: &mut Execution,
    ) -> Result<()> {
        if self.typed_assign_indexed(name, args, rhs, quiet, out)? {
            return Ok(());
        }
        let value = self.eval(rhs, None, out)?;
        let deletion = matches!(rhs,Expr::Matrix(rows) if rows.is_empty());
        self.assign_indexed_value(name, args, value, deletion, quiet, out)
    }
    /// The indexed write once the right side is evaluated.
    pub(super) fn assign_indexed_value(
        &mut self,
        name: &str,
        args: &[Expr],
        value: Value,
        deletion: bool,
        quiet: bool,
        out: &mut Execution,
    ) -> Result<()> {
        if matches!(value, Value::Container(crate::Array::Cell(_))) && !self.contains_array(name) {
            let value = value.array(&self.cancel)?;
            let updated = self.cel_assign(None, args, false, value, deletion, out)?;
            return self.assign_workspace_array(name, updated, quiet, out);
        }
        if self.complex_variables.contains_key(name)
            || (!self.contains_array(name) && matches!(value, Value::Complex(_)))
        {
            let values = value.complex(&self.cancel)?;
            return self.assign_typed::<crate::ComplexMatrix>(
                name, args, values, deletion, quiet, out, None,
            );
        }
        if matches!(value, Value::Complex(_)) {
            return Err("Indexed complex assignment into a real/logical variable requires explicit complex conversion first".into());
        }
        if value.is_character() && !self.contains_array(name) {
            // MATLAB creates char here; indexed char growth is not implemented.
            return Err(
                "Indexed char assignment needs an existing variable; assign the char array first"
                    .into(),
            );
        }
        if self.logical_variables.contains_key(name)
            || (!self.variables.contains_key(name) && matches!(value, Value::Logical(_)))
        {
            if value.is_character() {
                return Err(super::character_values::CHAR_TO_LOGICAL.into());
            }
            let values = value.logical(&self.cancel)?;
            self.assign_typed::<LogicalMatrix>(name, args, values, deletion, quiet, out, None)
        } else {
            let values = value.matrix(&self.cancel)?;
            self.assign_typed::<Matrix>(name, args, values, deletion, quiet, out, None)
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn assign_typed<T: StoredArray>(
        &mut self,
        name: &str,
        args: &[Expr],
        values: T,
        deletion: bool,
        quiet: bool,
        out: &mut Execution,
        prepared: Option<(Indices, Option<Indices>)>,
    ) -> Result<()> {
        let original = T::lookup(self, name).map(|m| (m.rows(), m.cols()));
        let old = original.unwrap_or((0, 0));
        if deletion && original.is_none() {
            return Err("Deletion requires an existing variable".into());
        }
        let (mut rows, mut columns) = if let Some(prepared) = prepared {
            prepared
        } else {
            let bound = if args.len() == 1 {
                old.0 * old.1
            } else {
                old.0
            };
            let (rows, _) = self.subscript_with_limit(
                &args[0],
                original.map(|_| bound),
                if deletion { bound } else { MAX_ELEMENTS },
                out,
            )?;
            let columns = if args.len() == 2 {
                Some(
                    self.subscript_with_limit(
                        &args[1],
                        original.map(|_| old.1),
                        if deletion { old.1 } else { MAX_ELEMENTS },
                        out,
                    )?
                    .0,
                )
            } else {
                None
            };

            (rows, columns)
        };
        if deletion {
            let result = self.delete_indexed::<T>(name, args, &rows, columns.as_ref())?;
            return self.assign_array(name, result, quiet, out);
        }

        if original.is_none() && args.len() == 1 && self.colon_subscript(&args[0]) {
            if !values.is_empty() && values.rows() != 1 && values.cols() != 1 {
                return Err("A new variable assigned through A(:) requires a vector".into());
            }
            rows = all(values.len());
        }
        // Only a truly 0x0 destination infers missing colon dimensions. A 0xN
        // or Nx0 array retains its known dimensions, matching MATLAB.
        if old == (0, 0)
            && let Some(columns) = &mut columns
        {
            let row_colon = self.colon_subscript(&args[0]);
            let col_colon = self.colon_subscript(&args[1]);
            if row_colon && col_colon {
                rows = all(values.rows());
                *columns = all(values.cols());
            } else if row_colon {
                rows = all(if columns.len() == 0 {
                    0
                } else if columns.len() == 1 && (values.rows() == 1 || values.cols() == 1) {
                    values.len()
                } else {
                    values.rows()
                });
            } else if col_colon {
                *columns = all(if rows.len() == 0 {
                    0
                } else if rows.len() == 1 && (values.rows() == 1 || values.cols() == 1) {
                    values.len()
                } else {
                    values.cols()
                });
            }
        }
        let count = if let Some(columns) = &columns {
            Matrix::checked_len(rows.len(), columns.len())?
        } else {
            rows.len()
        };
        if values.len() != 1 {
            if count != values.len() {
                return Err(
                    "Indexed assignment requires matching element counts or a scalar".into(),
                );
            }
            if count != 0
                && let Some(columns) = &columns
            {
                let selected: Vec<_> = [rows.len(), columns.len()]
                    .into_iter()
                    .filter(|n| *n != 1)
                    .collect();
                let source: Vec<_> = [values.rows(), values.cols()]
                    .into_iter()
                    .filter(|n| *n != 1)
                    .collect();
                if selected != source {
                    return Err("Subscripted assignment dimensions must agree".into());
                }
            }
        }
        let extent = self.index_extent(&rows)?;
        let shape = if let Some(columns) = &columns {
            (old.0.max(extent), old.1.max(self.index_extent(columns)?))
        } else if extent > old.0 * old.1 || original.is_none() {
            if old.0 > 1 && old.1 > 1 {
                return Err("Cannot grow a matrix through one ambiguous linear subscript; use row and column indices".into());
            }
            if old.1 == 1 && old.0 != 1 {
                (extent, 1)
            } else {
                (1, extent)
            }
        } else {
            old
        };
        let len = self.write_budget::<T>(name, shape)?;
        let full = rows.contiguous() == Some(0)
            && if let Some(columns) = &columns {
                rows.len() == shape.0 && columns.contiguous() == Some(0) && columns.len() == shape.1
            } else {
                rows.len() == len
            };
        // Complete replacement can reuse the RHS allocation, including aliases.
        if full && values.len() == len {
            let result = values.reshape(shape.0, shape.1)?;
            return self.assign_array(name, result, quiet, out);
        }
        let complete = if let Some(columns) = &columns {
            covers(&rows, shape.0) && covers(columns, shape.1)
        } else {
            covers(&rows, len)
        };
        if len == 0 || complete {
            // Complete forward/reverse stride permutations do not need old data.
            let data = if values.len() == 1 {
                let value = values.data()[0];
                self.replacement_buffer::<T>(len, |_| value)?
            } else if columns.is_none() && matches!(rows, Indices::Stride { step: -1, .. }) {
                // Dispatch before the hot loop so a full reverse can vectorize
                // without inspecting the expanded numeric/logical index enum.
                self.replacement_buffer::<T>(len, |i| values.data()[len - 1 - i])?
            } else {
                self.replacement_buffer::<T>(len, |i| {
                    if let Some(columns) = &columns {
                        values.data()[rows.at(i % shape.0) + columns.at(i / shape.0) * shape.0]
                    } else {
                        values.data()[rows.at(i)]
                    }
                })?
            };
            let result = T::from_vec(shape.0, shape.1, data)?;
            return self.assign_array(name, result, quiet, out);
        }
        if count == 0 && shape == old && original.is_some() {
            if !quiet {
                let text = T::lookup(self, name)
                    .expect("existing destination")
                    .echo(name, &self.display)?;
                self.emit(out, text)?;
            }
            return Ok(());
        }
        let address = |i: usize| -> usize {
            if let Some(columns) = &columns {
                rows.at(i % rows.len()) + shape.0 * columns.at(i / rows.len())
            } else {
                rows.at(i)
            }
        };
        let value = |i: usize| values.data()[if values.len() == 1 { 0 } else { i }];

        // Short writes commit only after all validation and cancellation checks.
        // The bounded commit has no fallible work or cancellation point inside.
        if count <= CHUNK && shape == old && original.is_some() {
            let changes: Vec<_> = if let Some(start) = rows.contiguous()
                && columns.as_ref().is_none_or(|c| c.len() == 1)
            {
                let offset = start + columns.as_ref().map_or(0, |c| c.at(0) * shape.0);
                (0..count).map(|i| (offset + i, value(i))).collect()
            } else {
                (0..count).map(|i| (address(i), value(i))).collect()
            };
            self.check_cancel()?;
            if T::lookup_mut(self, name)
                .expect("existing destination")
                .write_unique(&changes)
            {
                if !quiet {
                    let text = T::lookup(self, name)
                        .expect("existing destination")
                        .echo(name, &self.display)?;
                    self.emit(out, text)?;
                }
                return Ok(());
            }
        }
        if count > CHUNK
            && count <= len / 2
            && count <= 2 * 1024 * 1024
            && shape == old
            && columns.as_ref().is_none_or(|c| c.len() <= 1024)
            && let Some(start) = rows.contiguous()
        {
            let regions: Vec<_> = if let Some(columns) = &columns {
                (0..columns.len())
                    .map(|c| {
                        let first = columns.at(c) * shape.0 + start;
                        first..first + rows.len()
                    })
                    .collect()
            } else {
                std::iter::once(start..start + rows.len()).collect()
            };
            let cancel = self.cancel.clone();
            let check = || {
                if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    Err("Calculation cancelled".into())
                } else {
                    Ok(())
                }
            };
            if T::lookup_mut(self, name)
                .expect("existing destination")
                .write_unique_regions(&regions, values.data(), check)?
            {
                if !quiet {
                    let text = T::lookup(self, name)
                        .expect("existing destination")
                        .echo(name, &self.display)?;
                    self.emit(out, text)?;
                }
                return Ok(());
            }
        }
        let mut data = self.copy_for_write::<T>(name, shape, len)?;
        if let Some(start) = rows.contiguous() {
            if let Some(columns) = &columns {
                for c in 0..columns.len() {
                    let offset = columns.at(c) * shape.0 + start;
                    self.write_contiguous::<T>(
                        &mut data[offset..offset + rows.len()],
                        &values,
                        c * rows.len(),
                    )?;
                }
            } else {
                self.write_contiguous::<T>(&mut data[start..start + rows.len()], &values, 0)?;
            }
        } else {
            for first in (0..count).step_by(CHUNK) {
                self.check_cancel()?;
                for i in first..(first + CHUNK).min(count) {
                    data[address(i)] = value(i);
                }
            }
        }
        let result = T::from_vec(shape.0, shape.1, data)?;
        self.assign_array(name, result, quiet, out)
    }

    fn delete_indexed<T: StoredArray>(
        &mut self,
        name: &str,
        args: &[Expr],
        rows: &Indices,
        columns: Option<&Indices>,
    ) -> Result<T> {
        let old = &T::lookup(self, name).expect("existing destination");
        let (nr, nc, len) = (old.rows(), old.cols(), old.len());
        if args.len() == 1 && self.colon_subscript(&args[0]) {
            self.write_budget::<T>(name, (0, 0))?;
            return T::from_vec(0, 0, Vec::new());
        }
        let (indices, bound, dimension) = if let Some(columns) = columns {
            if self.colon_subscript(&args[1]) {
                (rows, nr, 0)
            } else if self.colon_subscript(&args[0]) {
                (columns, nc, 1)
            } else {
                return Err("A null assignment requires at least one ':' subscript".into());
            }
        } else {
            (rows, len, 2)
        };
        let mut removed = vec![false; bound];
        let mut nremoved = 0;
        for first in (0..indices.len()).step_by(CHUNK) {
            self.check_cancel()?;
            for i in first..(first + CHUNK).min(indices.len()) {
                let idx = indices.at(i);
                if !removed[idx] {
                    removed[idx] = true;
                    nremoved += 1;
                }
            }
        }
        let shape = match dimension {
            0 => (nr - nremoved, nc),
            1 => (nr, nc - nremoved),
            _ if len == 0 => (nr, nc),
            _ if nc == 1 && nr != 1 => (len - nremoved, 1),
            _ => (1, len - nremoved),
        };
        let count = self.write_budget::<T>(name, shape)?;
        if count == 0 {
            return T::from_vec(shape.0, shape.1, Vec::new());
        }
        let old = &T::lookup(self, name).expect("existing destination");
        if nremoved == 0 {
            return old.reshape(shape.0, shape.1);
        }
        let mut data = self.stored_buffer::<T>(count, false)?;
        if dimension == 1 {
            for (c, &skip) in removed.iter().enumerate() {
                self.check_cancel()?;
                if !skip {
                    for chunk in old.data()[c * nr..(c + 1) * nr].chunks(CHUNK) {
                        self.check_cancel()?;
                        data.extend_from_slice(chunk);
                    }
                }
            }
        } else {
            for first in (0..len).step_by(CHUNK) {
                self.check_cancel()?;
                for i in first..(first + CHUNK).min(len) {
                    if !removed[if dimension == 0 { i % nr } else { i }] {
                        data.push(old.data()[i]);
                    }
                }
            }
        }
        T::from_vec(shape.0, shape.1, data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn replacement_cancels_after_work_starts_without_publishing_data() {
        for threads in [1, 8] {
            rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap()
                .install(|| {
                    let mut engine = Engine::default();
                    engine.execute("A=[1 2 3];").unwrap();
                    let original = engine.variables["A"].clone();
                    let cancel = engine.cancellation();
                    let visited = AtomicUsize::new(0);
                    let result = engine.replacement_buffer::<Matrix>(1024 * 1024, |_| {
                        if visited.fetch_add(1, Ordering::Relaxed) == CHUNK - 1 {
                            cancel.store(true, Ordering::Relaxed);
                        }
                        7.
                    });
                    assert!(result.unwrap_err().contains("cancelled"));
                    assert!(visited.load(Ordering::Relaxed) >= CHUNK);
                    assert!(visited.load(Ordering::Relaxed) < 1024 * 1024);
                    assert_eq!(
                        engine.variables["A"].data().as_ptr(),
                        original.data().as_ptr()
                    );
                    assert_eq!(engine.variables["A"].data(), &[1., 2., 3.]);
                });
        }
    }
}

#[cfg(test)]
mod complex_cancellation_tests {
    use super::*;
    use crate::{Complex64, ComplexMatrix};
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn complex_replacement_cancels_after_copy_progress_and_retries_without_publishing() {
        // Inject through the existing replacement value callback, with no test
        // branch in production kernels and no timing-dependent cancellation.
        for workers in [1, 8] {
            rayon::ThreadPoolBuilder::new()
                .num_threads(workers)
                .build()
                .unwrap()
                .install(|| {
                    let mut engine = Engine::default();
                    engine.execute("Z=complex([1 2],[-0 3]);Alias=Z;").unwrap();
                    let original = engine.complex_variables["Z"].clone();
                    let before = original
                        .data()
                        .iter()
                        .map(|v| (v.re.to_bits(), v.im.to_bits()))
                        .collect::<Vec<_>>();
                    let cancel = engine.cancellation();
                    let visited = AtomicUsize::new(0);
                    let count = 1024 * 1024;
                    let result = engine.replacement_buffer::<ComplexMatrix>(count, |i| {
                        if visited.fetch_add(1, Ordering::Relaxed) == CHUNK - 1 {
                            cancel.store(true, Ordering::Relaxed);
                        }
                        Complex64::new(i as f64, -(i as f64))
                    });
                    assert!(result.unwrap_err().contains("cancelled"));
                    assert!(cancel.load(Ordering::Relaxed));
                    assert!((CHUNK..count).contains(&visited.load(Ordering::Relaxed)));
                    for name in ["Z", "Alias"] {
                        assert!(engine.complex_variables[name].shares_storage(&original));
                        assert_eq!(
                            engine.complex_variables[name]
                                .data()
                                .iter()
                                .map(|v| (v.re.to_bits(), v.im.to_bits()))
                                .collect::<Vec<_>>(),
                            before
                        );
                    }
                    cancel.store(false, Ordering::Relaxed);
                    let retry = engine
                        .replacement_buffer::<ComplexMatrix>(count, |i| {
                            Complex64::new(i as f64, -(i as f64))
                        })
                        .unwrap();
                    assert_eq!(retry.len(), count);
                    for (i, value) in retry.iter().enumerate() {
                        assert_eq!(
                            (value.re.to_bits(), value.im.to_bits()),
                            ((i as f64).to_bits(), (-(i as f64)).to_bits())
                        );
                    }
                    // The real script commit path is also retried after cancellation.
                    engine.execute("Z(:)=complex(7,-8);").unwrap();
                    assert!(
                        engine.complex_variables["Z"]
                            .data()
                            .iter()
                            .all(|v| *v == Complex64::new(7., -8.))
                    );
                    assert!(engine.complex_variables["Alias"].shares_storage(&original));
                });
        }
    }
}
