//! Compact index descriptions: colon/ranges never need an index-sized allocation.
//! Explicit numeric subscripts retain their immutable buffer after validation.
use super::{Engine, Execution, Value, stored_array::StoredArray};
use crate::{Matrix, Result, matrix::MAX_ELEMENTS, syntax::Expr};
use rayon::prelude::*;

const CHUNK: usize = 8192;
// Measured benefit at 512 Ki output elements; smaller gathers stay sequential.

pub(super) enum Indices {
    Stride {
        start: usize,
        step: isize,
        len: usize,
    },
    Explicit(Matrix),
    Mask(Vec<u32>, bool),
    LogicalStride {
        start: usize,
        len: usize,
        vector: bool,
    },
}

impl Indices {
    pub(super) fn len(&self) -> usize {
        match self {
            Self::Stride { len, .. } => *len,
            Self::Explicit(m) => m.len(),
            Self::Mask(v, _) => v.len(),
            Self::LogicalStride { len, .. } => *len,
        }
    }

    pub(super) fn at(&self, i: usize) -> usize {
        match self {
            Self::Stride { start, step, .. } => (*start as isize + *step * i as isize) as usize,
            Self::Explicit(m) => m.data()[i] as usize - 1,
            Self::Mask(v, _) => v[i] as usize,
            Self::LogicalStride { start, .. } => *start + i,
        }
    }

    pub(super) fn contiguous(&self) -> Option<usize> {
        match self {
            Self::Stride { start, step: 1, .. } => Some(*start),
            Self::LogicalStride { start, .. } => Some(*start),
            Self::Mask(v, _) if v.is_empty() || v.last().unwrap() - v[0] + 1 == v.len() as u32 => {
                Some(v.first().copied().unwrap_or(0) as usize)
            }
            _ => None,
        }
    }
}

impl Engine {
    /// `:` or a char `':'` (literal or variable): both select every element.
    pub(super) fn colon_subscript(&self, arg: &Expr) -> bool {
        match arg {
            Expr::All => true,
            Expr::String(s) => s == ":",
            Expr::Name(name) => matches!(
                self.container_variables.get(name),
                Some(crate::Array::Character(a)) if a.units() == [u16::from(b':')]
            ),
            _ => false,
        }
    }

    fn mask_indices(&mut self, mask: &crate::LogicalMatrix, maximum: usize) -> Result<Indices> {
        self.check_cancel()?;
        let summarize = |(block, bytes): (usize, &[u8])| -> Result<(usize, usize, usize)> {
            self.check_cancel()?;
            let count = bytes.iter().map(|v| *v as usize).sum::<usize>();
            if count == 0 {
                return Ok((0, 0, 0));
            }
            let first = if count == bytes.len() {
                0
            } else {
                bytes.iter().position(|v| *v != 0).unwrap()
            };
            let last = if count == bytes.len() {
                bytes.len() - 1
            } else {
                bytes.iter().rposition(|v| *v != 0).unwrap()
            };
            Ok((count, block * CHUNK + first, block * CHUNK + last))
        };
        let parallel = mask.len() >= 4 * 1024 * 1024 && rayon::current_num_threads() > 1;
        let chunks = if parallel {
            mask.data()
                .par_chunks(CHUNK)
                .enumerate()
                .map(summarize)
                .collect::<Result<Vec<_>>>()?
        } else {
            mask.data()
                .chunks(CHUNK)
                .enumerate()
                .map(summarize)
                .collect::<Result<Vec<_>>>()?
        };
        let count = chunks.iter().map(|c| c.0).sum::<usize>();
        let first = chunks.iter().find(|c| c.0 != 0).map_or(0, |c| c.1);
        let last = chunks.iter().rfind(|c| c.0 != 0).map_or(0, |c| c.2);
        if count > 0 && last >= maximum {
            return Err(self.raise(
                "Logical index contains a true value outside the array bounds".into(),
                "MATLAB:badsubscript".into(),
            ));
        }
        let vector = mask.rows() == 1 || mask.cols() == 1;
        if count == 0 || last - first + 1 == count {
            return Ok(Indices::LogicalStride {
                start: first,
                len: count,
                vector,
            });
        }
        let mut positions = vec![0u32; count];
        let mut rest = positions.as_mut_slice();
        let outputs: Vec<_> = chunks
            .iter()
            .map(|c| {
                let (head, tail) = std::mem::take(&mut rest).split_at_mut(c.0);
                rest = tail;
                head
            })
            .collect();
        let fill = |(block, output): (usize, &mut [u32])| -> Result<()> {
            self.check_cancel()?;
            let mut next = 0;
            let start = block * CHUNK;
            for (i, &v) in mask.data()[start..(start + CHUNK).min(mask.len())]
                .iter()
                .enumerate()
            {
                if v != 0 {
                    output[next] = (start + i) as u32;
                    next += 1;
                }
            }
            Ok(())
        };
        if parallel {
            outputs.into_par_iter().enumerate().try_for_each(fill)?;
        } else {
            outputs.into_iter().enumerate().try_for_each(fill)?;
        }
        self.check_cancel()?;
        Ok(Indices::Mask(positions, vector))
    }
    /// `start:step:stop` as a value: integer or single operands keep their
    /// class, character operands make a character row.
    pub(super) fn range_value(
        &mut self,
        start: &Expr,
        step: Option<&Expr>,
        stop: &Expr,
        end: Option<usize>,
        out: &mut Execution,
    ) -> Result<Value> {
        let (first, step, last) = self.range_operands(start, step, stop, end, out)?;
        if typed_range_operands(&first, step.as_ref(), &last) {
            return self.typed_range(first, step, last);
        }
        let (start, step, count, character) = self.range_scalars(first, step, last)?;
        if character {
            return self.character_range(start, step, count);
        }
        Ok(Value::Matrix(self.range_matrix(start, step, count)?))
    }
    pub(super) fn range_parameters(
        &mut self,
        start: &Expr,
        step: Option<&Expr>,
        stop: &Expr,
        end: Option<usize>,
        out: &mut Execution,
    ) -> Result<(f64, f64, usize, bool)> {
        let (first, step, last) = self.range_operands(start, step, stop, end, out)?;
        self.range_scalars(first, step, last)
    }
    /// Colon operands in evaluation order: start, stop, then step.
    pub(super) fn range_operands(
        &mut self,
        start: &Expr,
        step: Option<&Expr>,
        stop: &Expr,
        end: Option<usize>,
        out: &mut Execution,
    ) -> Result<(Value, Option<Value>, Value)> {
        let first = self.eval(start, end, out)?;
        let last = self.eval(stop, end, out)?;
        let step = step.map(|s| self.eval(s, end, out)).transpose()?;
        Ok((first, step, last))
    }
    /// Start, step, element count and whether any operand is character, for
    /// double/logical/character colon operands.
    pub(super) fn range_scalars(
        &self,
        first: Value,
        step: Option<Value>,
        last: Value,
    ) -> Result<(f64, f64, usize, bool)> {
        let character = first.is_character()
            || last.is_character()
            || step.as_ref().is_some_and(Value::is_character);
        let start = first.matrix(&self.cancel)?.as_scalar()?;
        let stop = last.matrix(&self.cancel)?.as_scalar()?;
        let step = match step {
            Some(s) => s.matrix(&self.cancel)?.as_scalar()?,
            None => 1.0,
        };
        if !start.is_finite() || !stop.is_finite() || !step.is_finite() || step == 0.0 {
            return Err("Range bounds must be finite and step must be nonzero".into());
        }
        let span = (stop - start) / step;
        let count = if span < 0.0 {
            0.0
        } else {
            (span + 4.0 * f64::EPSILON * span.abs().max(1.0)).floor() + 1.0
        };
        if count > MAX_ELEMENTS as f64 {
            return Err("Range exceeds matrix element limit".into());
        }
        if character {
            crate::CharacterRow::checked_len(1, count as usize)?;
            if start.fract() != 0.
                || stop.fract() != 0.
                || step.fract() != 0.
                || !(0. ..=65535.).contains(&start)
                || !(0. ..=65535.).contains(&stop)
            {
                return Err(
                    "Character ranges require integer UTF-16 bounds 0..65535 and an integer step"
                        .into(),
                );
            }
        }
        Ok((start, step, count as usize, character))
    }

    pub(super) fn range_matrix(&self, start: f64, step: f64, count: usize) -> Result<Matrix> {
        let mut data = Vec::with_capacity(count);
        for first in (0..count).step_by(CHUNK) {
            self.check_cancel()?;
            data.extend((first..(first + CHUNK).min(count)).map(|i| start + i as f64 * step));
        }
        Matrix::from_column_vec(1, count, data)
    }

    fn subscript(
        &mut self,
        arg: &Expr,
        bound: usize,
        out: &mut Execution,
    ) -> Result<(Indices, (usize, usize))> {
        self.subscript_with_limit(arg, Some(bound), bound, out)
    }

    pub(super) fn subscript_with_limit(
        &mut self,
        arg: &Expr,
        bound: Option<usize>,
        maximum: usize,
        out: &mut Execution,
    ) -> Result<(Indices, (usize, usize))> {
        if self.colon_subscript(arg) {
            let bound = bound.unwrap_or(0);
            return Ok((
                Indices::Stride {
                    start: 0,
                    step: 1,
                    len: bound,
                },
                (bound, 1),
            ));
        }
        let values = if let Expr::Range(start, step, stop) = arg {
            let (start, step, len, _) =
                self.range_parameters(start, step.as_deref(), stop, bound, out)?;
            // Charge the logical range just as eval would, even without its allocation.
            self.charge(len)?;
            self.check_cancel()?;
            if len == 0 {
                return Ok((
                    Indices::Stride {
                        start: 0,
                        step: 1,
                        len,
                    },
                    (1, 0),
                ));
            }
            if start.fract() == 0.0 && (len == 1 || step.fract() == 0.0) {
                let last = start + (len - 1) as f64 * step;
                if start < 1.0 || start > maximum as f64 || last < 1.0 || last > maximum as f64 {
                    return Err(self.raise(
                        format!("Index must be an integer between 1 and {maximum}"),
                        "MATLAB:badsubscript".into(),
                    ));
                }
                // For two or more values both endpoints bound the step and all
                // intermediate integer arithmetic to +/- MAX_ELEMENTS.
                return Ok((
                    Indices::Stride {
                        start: start as usize - 1,
                        step: if len == 1 { 1 } else { step as isize },
                        len,
                    },
                    (1, len),
                ));
            }
            self.range_matrix(start, step, len)?
        } else {
            match self.eval(arg, bound, out)? {
                Value::Logical(mask) => {
                    let indices = self.mask_indices(&mask, maximum)?;
                    let count = indices.len();
                    return Ok((indices, (count, 1)));
                }
                value if value.is_colon_character() => {
                    let len = bound.unwrap_or(0);
                    let all = Indices::Stride {
                        start: 0,
                        step: 1,
                        len,
                    };
                    return Ok((all, (len, 1)));
                }
                value => value.matrix(&self.cancel)?,
            }
        };
        for chunk in values.data().chunks(CHUNK) {
            self.check_cancel()?;
            for &x in chunk {
                if !x.is_finite() || x.fract() != 0.0 || x < 1.0 || x > maximum as f64 {
                    return Err(self.raise(
                        format!("Index must be an integer between 1 and {maximum}"),
                        "MATLAB:badsubscript".into(),
                    ));
                }
            }
        }
        let shape = (values.rows(), values.cols());
        if values.len() == 1 {
            Ok((
                Indices::Stride {
                    start: values.data()[0] as usize - 1,
                    step: 1,
                    len: 1,
                },
                shape,
            ))
        } else {
            Ok((Indices::Explicit(values), shape))
        }
    }

    fn append_slice<T: Copy>(&self, output: &mut Vec<T>, input: &[T]) -> Result<()> {
        for chunk in input.chunks(CHUNK) {
            self.check_cancel()?;
            output.extend_from_slice(chunk);
        }
        Ok(())
    }

    fn parallel_gather<T: StoredArray>(
        &self,
        m: &T,
        rows: &Indices,
        columns: Option<&Indices>,
        count: usize,
    ) -> Result<Vec<T::Element>> {
        let mut data = self.stored_buffer::<T>(count, true)?;
        data.par_chunks_mut(CHUNK)
            .enumerate()
            .try_for_each(|(chunk, output)| -> Result<()> {
                self.check_cancel()?;
                if let Some(columns) = columns {
                    // Split only at column boundaries, avoiding division for every sample.
                    let mut first = chunk * CHUNK;
                    let mut output = output;
                    while !output.is_empty() {
                        let row = first % rows.len();
                        let col = first / rows.len();
                        let n = output.len().min(rows.len() - row);
                        let (head, tail) = output.split_at_mut(n);
                        let offset = columns.at(col) * m.rows();
                        if let Some(start) = rows.contiguous() {
                            head.copy_from_slice(
                                &m.data()[offset + start + row..offset + start + row + n],
                            );
                        } else {
                            for (i, value) in head.iter_mut().enumerate() {
                                *value = m.data()[offset + rows.at(row + i)];
                            }
                        }
                        first += n;
                        output = tail;
                    }
                } else {
                    for (i, value) in output.iter_mut().enumerate() {
                        *value = m.data()[rows.at(chunk * CHUNK + i)];
                    }
                }
                Ok(())
            })?;
        Ok(data)
    }

    pub(super) fn index_array<T: StoredArray>(
        &mut self,
        m: &T,
        args: &[Expr],
        out: &mut Execution,
        container_diagnostics: bool,
    ) -> Result<T> {
        if args.is_empty() || args.len() > 2 {
            return Err("Indexing requires one or two subscripts".into());
        }
        let bound = if args.len() == 1 { m.len() } else { m.rows() };
        let (rows, mut shape) = if container_diagnostics {
            self.cel_subscript(&args[0], bound, bound, out)?
        } else {
            self.subscript(&args[0], bound, out)?
        };
        let columns = if args.len() == 2 {
            let (columns, _) = if container_diagnostics {
                self.cel_subscript(&args[1], m.cols(), m.cols(), out)?
            } else {
                self.subscript(&args[1], m.cols(), out)?
            };
            shape = (rows.len(), columns.len());
            Some(columns)
        } else {
            if !self.colon_subscript(&args[0])
                && (m.rows() == 1 || m.cols() == 1)
                && match &rows {
                    Indices::Mask(_, vector) | Indices::LogicalStride { vector, .. } => *vector,
                    _ => shape.0 == 1 || shape.1 == 1,
                }
            {
                shape = if m.rows() == 1 {
                    (1, rows.len())
                } else {
                    (rows.len(), 1)
                };
            }
            None
        };
        let count = T::checked_len(shape.0, shape.1)?;
        // Reject oversized evaluation before allocating the result. eval charges
        // the returned matrix once; this is a preflight, not another charge.
        if self
            .evaluated_elements
            .saturating_add(count * T::BYTES.div_ceil(8))
            > 32 * 1024 * 1024
        {
            return Err("Statement exceeds its 256 MiB evaluation budget; split the calculation into smaller steps".into());
        }
        self.check_cancel()?;
        let contiguous = if let Some(columns) = &columns {
            if rows.contiguous() == Some(0) && rows.len() == m.rows() {
                columns.contiguous().map(|c| c * m.rows())
            } else {
                None
            }
        } else {
            rows.contiguous()
        };
        let stride = if columns.is_none() {
            match &rows {
                Indices::Stride { start, step, .. } if matches!(*step, 1 | 2) => {
                    Some((*start, *step as usize))
                }
                _ => None,
            }
        } else {
            None
        };
        if contiguous == Some(0) && count == m.len() {
            return m.reshape(shape.0, shape.1);
        }
        if contiguous.is_none() && count >= T::PAR_COPY && rayon::current_num_threads() > 1 {
            let data = self.parallel_gather(m, &rows, columns.as_ref(), count)?;
            let mut result = T::from_vec(shape.0, shape.1, data)?;
            if let Some((start, step)) = stride {
                result.inherit_stride(m, start, step);
            }
            return Ok(result);
        }
        let mut data = self.stored_buffer::<T>(count, false)?;
        if count == 0 {
            return T::from_vec(shape.0, shape.1, data);
        }
        if let Some(start) = contiguous {
            self.append_slice(&mut data, &m.data()[start..start + count])?;
        } else if let Some(columns) = &columns {
            for c in 0..columns.len() {
                self.check_cancel()?;
                let offset = columns.at(c) * m.rows();
                if let Some(start) = rows.contiguous() {
                    self.append_slice(
                        &mut data,
                        &m.data()[offset + start..offset + start + rows.len()],
                    )?;
                } else {
                    for first in (0..rows.len()).step_by(CHUNK) {
                        self.check_cancel()?;
                        data.extend(
                            (first..(first + CHUNK).min(rows.len()))
                                .map(|r| m.data()[offset + rows.at(r)]),
                        );
                    }
                }
            }
        } else {
            for first in (0..rows.len()).step_by(CHUNK) {
                self.check_cancel()?;
                data.extend((first..(first + CHUNK).min(rows.len())).map(|i| m.data()[rows.at(i)]));
            }
        }
        let mut result = T::from_vec(shape.0, shape.1, data)?;
        if let Some(start) = contiguous {
            result.inherit_contiguous(m, start);
        } else if let Some((start, step)) = stride {
            result.inherit_stride(m, start, step);
        }
        Ok(result)
    }
}

/// Whether an integer or single operand makes `start:step:stop` typed.
pub(super) fn typed_range_operands(first: &Value, step: Option<&Value>, last: &Value) -> bool {
    use super::typed_values::is_typed;
    is_typed(first) || is_typed(last) || step.is_some_and(is_typed)
}
