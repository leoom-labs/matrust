//! Borrowed vector sets. Serial production, bounded scratch, exact output counts.
use super::sort::{Key, check, compare_keys as compare, keys_equal, sort_lane, typed_key};
use crate::{
    Array, LogicalMatrix, Matrix, NumericClass, Result, TypedArray,
    array::{MissingPlacement, SortOrder, UniqueOrder},
};
use std::sync::atomic::AtomicBool;

const CHUNK: usize = 1024;
const PRIVATE_BYTES: usize = 256 * 1024 * 1024;
const ABSENT: u32 = u32::MAX;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetOperation {
    Union,
    Intersect,
    Difference,
    SymmetricDifference,
}

#[derive(Debug)]
pub struct SetResult {
    pub values: Option<Array>,
    pub first_a: Option<Matrix>,
    pub first_b: Option<Matrix>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Initialize,
    Sort,
    Count,
    Select,
    Reorder,
    Output,
}
fn checkpoint(cancel: &AtomicBool, phase: Phase) -> Result<()> {
    #[cfg(test)]
    tests::probe(cancel, phase);
    let _ = phase;
    check(cancel)
}

#[derive(Clone, Copy)]
enum Values<'a> {
    Real(&'a [f64]),
    Logical(&'a [u8]),
    Typed(&'a TypedArray),
}
impl Values<'_> {
    fn key(self, i: usize) -> Key {
        match self {
            Self::Real(a) => Key::Real(a[i]),
            Self::Logical(a) => Key::Real(f64::from(a[i])),
            Self::Typed(a) => typed_key(a, i),
        }
    }
    fn logical(self) -> bool {
        matches!(self, Self::Logical(_))
    }
    fn class(self) -> Option<NumericClass> {
        match self {
            Self::Typed(a) => Some(a.class()),
            _ => None,
        }
    }
}
struct Inputs<'a> {
    a: Values<'a>,
    b: Values<'a>,
    na: usize,
    nb: usize,
    row_a: bool,
    row_b: bool,
    rows_mode: bool,
    cols: usize,
}
impl Inputs<'_> {
    fn read(&self, i: usize) -> Key {
        if i < self.na {
            self.a.key(i)
        } else {
            self.b.key(i - self.na)
        }
    }
    fn read_at(&self, i: usize, col: usize) -> Key {
        if !self.rows_mode {
            return self.read(i);
        }
        if i < self.na {
            self.a.key(col * self.na + i)
        } else {
            self.b.key(col * self.nb + (i - self.na))
        }
    }
    fn cmp_items(&self, i: usize, j: usize) -> std::cmp::Ordering {
        if !self.rows_mode {
            return compare(
                self.read(i),
                self.read(j),
                SortOrder::Ascend,
                MissingPlacement::Last,
            );
        }
        for col in 0..self.cols {
            let ord = compare(
                self.read_at(i, col),
                self.read_at(j, col),
                SortOrder::Ascend,
                MissingPlacement::Last,
            );
            if ord != std::cmp::Ordering::Equal {
                return ord;
            }
        }
        std::cmp::Ordering::Equal
    }
    fn equal_items(&self, i: usize, j: usize) -> bool {
        if !self.rows_mode {
            return keys_equal(self.read(i), self.read(j));
        }
        (0..self.cols).all(|col| keys_equal(self.read_at(i, col), self.read_at(j, col)))
    }
    fn value_at(&self, r: Record, col: usize) -> Key {
        if !self.rows_mode {
            return self.value(r);
        }
        if r.a != ABSENT {
            self.a.key(col * self.na + r.a as usize)
        } else {
            self.b.key(col * self.nb + r.b as usize)
        }
    }
    fn value(&self, r: Record) -> Key {
        if r.a != ABSENT {
            self.a.key(r.a as usize)
        } else {
            self.b.key(r.b as usize)
        }
    }
    fn ordinal(&self, r: Record) -> usize {
        if r.a != ABSENT {
            r.a as usize
        } else {
            self.na + r.b as usize
        }
    }
}
fn admit(a: &Array) -> Result<Values<'_>> {
    Matrix::checked_len(a.rows(), a.cols())?;
    match a {
        Array::Numeric(a) => Ok(Values::Real(a.data())),
        Array::Logical(a) => Ok(Values::Logical(a.data())),
        Array::Typed(a) => Ok(Values::Typed(a)),
        _ => Err("Set operations support real numeric or logical inputs only".into()),
    }
}
#[derive(Clone, Copy)]
struct Request {
    operation: SetOperation,
    order: UniqueOrder,
    outputs: [bool; 3],
    units: usize,
}
#[derive(Clone, Copy)]
struct Record {
    a: u32,
    b: u32,
}
impl Default for Record {
    fn default() -> Self {
        Self {
            a: ABSENT,
            b: ABSENT,
        }
    }
}
fn select(mut r: Record, op: SetOperation) -> Option<Record> {
    let a = r.a != ABSENT;
    let b = r.b != ABSENT;
    let keep = match op {
        SetOperation::Union => {
            if a {
                r.b = ABSENT;
            }
            a || b
        }
        SetOperation::Intersect => a && b,
        SetOperation::Difference => a && !b,
        SetOperation::SymmetricDifference => a != b,
    };
    keep.then_some(r)
}
fn budget(bytes: Option<usize>) -> Result<usize> {
    bytes
        .filter(|&b| b <= PRIVATE_BYTES)
        .ok_or_else(|| "Set operations exceed the 256 MiB private data-buffer budget".into())
}
fn bytes<T>(a: &Vec<T>) -> usize {
    a.capacity() * std::mem::size_of::<T>()
}
fn allocate<T: Default + Clone>(n: usize, live: usize, cancel: &AtomicBool) -> Result<Vec<T>> {
    check(cancel)?;
    budget(
        n.checked_mul(std::mem::size_of::<T>())
            .and_then(|b| live.checked_add(b)),
    )?;
    let mut a = Vec::new();
    a.try_reserve_exact(n)
        .map_err(|_| "Cannot allocate set-operation buffer")?;
    // Charge actual retained capacity as well as the requested preallocation.
    budget(live.checked_add(bytes(&a)))?;
    while a.len() < n {
        checkpoint(cancel, Phase::Initialize)?;
        a.resize((a.len() + CHUNK).min(n), T::default());
    }
    check(cancel)?;
    Ok(a)
}
fn counts(records: impl Iterator<Item = Record>, cancel: &AtomicBool) -> Result<[usize; 3]> {
    let mut counts = [0; 3];
    for (i, r) in records.enumerate() {
        if i.is_multiple_of(CHUNK) {
            checkpoint(cancel, Phase::Count)?;
        }
        counts[0] += 1;
        counts[1] += usize::from(r.a != ABSENT);
        counts[2] += usize::from(r.b != ABSENT);
    }
    checkpoint(cancel, Phase::Count)?;
    Ok(counts)
}
fn output_class(inputs: &Inputs<'_>, op: SetOperation) -> Result<(bool, Option<NumericClass>)> {
    if let (Some(ca), Some(cb)) = (inputs.a.class(), inputs.b.class())
        && ca != cb
    {
        return Err("Set operations require inputs of the same integer or single class".into());
    }
    let typed = if op == SetOperation::Difference {
        inputs.a.class()
    } else {
        inputs.a.class().or_else(|| inputs.b.class())
    };
    let logical = typed.is_none()
        && inputs.a.logical()
        && (op == SetOperation::Difference || inputs.b.logical());
    Ok((logical, typed))
}
fn output_admission(
    counts: [usize; 3],
    rows_mode: bool,
    cols: usize,
    request: Request,
    logical: bool,
    typed: Option<NumericClass>,
) -> Result<usize> {
    let mut units = 0usize;
    let mut size = 0usize;
    let elem_size = typed.map_or(if logical { 1 } else { 8 }, NumericClass::size);
    for (i, &n) in counts.iter().enumerate() {
        if request.outputs[i] {
            let c = if i == 0 && rows_mode { cols } else { 1 };
            let elems = Matrix::checked_len(n, c)?;
            units = units
                .checked_add(elems)
                .ok_or("Set output count overflow")?;
            size = size
                .checked_add(
                    elems
                        .checked_mul(if i == 0 { elem_size } else { 8 })
                        .ok_or("Set output size overflow")?,
                )
                .ok_or("Set output size overflow")?;
        }
    }
    if units > request.units {
        return Err("Statement exceeds its 256 MiB evaluation budget; split the calculation into smaller steps".into());
    }
    Ok(size)
}
fn scan(
    indices: &[u32],
    inputs: &Inputs<'_>,
    request: Request,
    phase: Phase,
    cancel: &AtomicBool,
    mut visit: impl FnMut(Record),
) -> Result<()> {
    checkpoint(cancel, phase)?;
    let mut start = 0;
    while start < indices.len() {
        let first = indices[start] as usize;
        let mut r = Record::default();
        let mut end = start;
        loop {
            if end.is_multiple_of(CHUNK) {
                checkpoint(cancel, phase)?;
            }
            let at = indices[end] as usize;
            if at < inputs.na {
                r.a = r.a.min(at as u32);
            } else {
                r.b = r.b.min((at - inputs.na) as u32);
            }
            end += 1;
            // IEEE equality, not sort equivalence: each NaN is a separate run.
            if end == indices.len() || !inputs.equal_items(first, indices[end] as usize) {
                break;
            }
        }
        if let Some(r) = select(r, request.operation) {
            visit(r);
        }
        start = end;
    }
    checkpoint(cancel, phase)
}
fn logical_records(
    inputs: &Inputs<'_>,
    request: Request,
    cancel: &AtomicBool,
) -> Result<([Record; 2], usize)> {
    let mut first = [[ABSENT; 2]; 2];
    for (side, values) in [inputs.a, inputs.b].into_iter().enumerate() {
        let Values::Logical(data) = values else {
            unreachable!()
        };
        for (i, &v) in data.iter().enumerate() {
            if i.is_multiple_of(CHUNK) {
                checkpoint(cancel, Phase::Count)?;
            }
            first[side][usize::from(v)] = first[side][usize::from(v)].min(i as u32);
            if first[side].iter().all(|&x| x != ABSENT) {
                break;
            }
        }
    }
    checkpoint(cancel, Phase::Count)?;
    let mut records = [Record::default(); 2];
    let mut k = 0;
    for (&a, &b) in first[0].iter().zip(&first[1]) {
        if let Some(r) = select(Record { a, b }, request.operation) {
            records[k] = r;
            k += 1;
        }
    }
    if k == 2
        && request.order == UniqueOrder::Stable
        && inputs.ordinal(records[0]) > inputs.ordinal(records[1])
    {
        records.swap(0, 1);
    }
    Ok((records, k))
}
fn finish(
    inputs: &Inputs<'_>,
    records: &[Record],
    permutation: Option<&[u32]>,
    mut live: usize,
    request: Request,
    cancel: &AtomicBool,
) -> Result<SetResult> {
    let lengths = counts(records.iter().copied(), cancel)?;
    let (logical, typed) = output_class(inputs, request.operation)?;
    let output_bytes = output_admission(
        lengths,
        inputs.rows_mode,
        inputs.cols,
        request,
        logical,
        typed,
    )?;
    budget(live.checked_add(output_bytes))?;
    checkpoint(cancel, Phase::Output)?;
    let mut real = None;
    let mut ints = None;
    let mut bits = None;
    let mut ia = None;
    let mut ib = None;
    let out_rows_len = lengths[0];
    let out_val_len = if inputs.rows_mode {
        out_rows_len * inputs.cols
    } else {
        out_rows_len
    };
    if request.outputs[0] {
        if let Some(class) = typed {
            if class.is_integer() {
                let a = allocate::<i128>(out_val_len, live, cancel)?;
                live += bytes(&a);
                ints = Some(a);
            } else {
                let a = allocate::<f64>(out_val_len, live, cancel)?;
                live += bytes(&a);
                real = Some(a);
            }
        } else if logical {
            let a = allocate::<u8>(out_val_len, live, cancel)?;
            live += bytes(&a);
            bits = Some(a);
        } else {
            let a = allocate::<f64>(out_val_len, live, cancel)?;
            live += bytes(&a);
            real = Some(a);
        }
    }
    if request.outputs[1] {
        let a = allocate::<f64>(lengths[1], live, cancel)?;
        live += bytes(&a);
        ia = Some(a);
    }
    if request.outputs[2] {
        let a = allocate::<f64>(lengths[2], live, cancel)?;
        live += bytes(&a);
        ib = Some(a);
    }
    budget(Some(live))?;
    let (mut ai, mut bi) = (0, 0);
    let cols = if inputs.rows_mode { inputs.cols } else { 1 };
    for i in 0..records.len() {
        if i.is_multiple_of(CHUNK) {
            checkpoint(cancel, Phase::Output)?;
        }
        let r = records[permutation.map_or(i, |p| p[i] as usize)];
        if let Some(a) = &mut real {
            for col in 0..cols {
                a[col * out_rows_len + i] = match inputs.value_at(r, col) {
                    Key::Real(v) => v,
                    Key::Integer(v) => v as f64,
                };
            }
        }
        if let (Some(a), Some(class)) = (&mut ints, typed) {
            for col in 0..cols {
                a[col * out_rows_len + i] = match inputs.value_at(r, col) {
                    Key::Integer(v) => v,
                    Key::Real(v) => class.from_f64(v),
                };
            }
        }
        if let Some(a) = &mut bits {
            for col in 0..cols {
                a[col * out_rows_len + i] =
                    u8::from(!keys_equal(inputs.value_at(r, col), Key::Real(0.)));
            }
        }
        if r.a != ABSENT {
            if let Some(a) = &mut ia {
                a[ai] = f64::from(r.a) + 1.;
            }
            ai += 1;
        }
        if r.b != ABSENT {
            if let Some(a) = &mut ib {
                a[bi] = f64::from(r.b) + 1.;
            }
            bi += 1;
        }
    }
    checkpoint(cancel, Phase::Output)?;
    let (rows, cols) = if inputs.rows_mode {
        (out_rows_len, inputs.cols)
    } else if inputs.row_a && (request.operation == SetOperation::Difference || inputs.row_b) {
        (1, out_rows_len)
    } else {
        (out_rows_len, 1)
    };
    let values = if let Some(class) = typed {
        if let Some(a) = ints {
            Some(TypedArray::from_ints(class, rows, cols, a.into_iter())?.into())
        } else {
            real.map(|a| TypedArray::from_reals(class, rows, cols, a.into_iter()).map(Into::into))
                .transpose()?
        }
    } else if let Some(a) = real {
        Some(Matrix::from_column_vec(rows, cols, a)?.into())
    } else if let Some(a) = bits {
        Some(LogicalMatrix::from_column_vec(rows, cols, a, cancel)?.into())
    } else {
        None
    };
    let first_a = ia
        .map(|a| Matrix::from_column_vec(lengths[1], 1, a))
        .transpose()?;
    let first_b = ib
        .map(|a| Matrix::from_column_vec(lengths[2], 1, a))
        .transpose()?;
    check(cancel)?;
    Ok(SetResult {
        values,
        first_a,
        first_b,
    })
}
fn run(inputs: &Inputs<'_>, request: Request, cancel: &AtomicBool) -> Result<SetResult> {
    if !request.outputs.iter().any(|&v| v) {
        return Ok(SetResult {
            values: None,
            first_a: None,
            first_b: None,
        });
    }
    if !inputs.rows_mode && inputs.a.logical() && inputs.b.logical() {
        let (records, n) = logical_records(inputs, request, cancel)?;
        return finish(inputs, &records[..n], None, 0, request, cancel);
    }
    let n = inputs
        .na
        .checked_add(inputs.nb)
        .ok_or("Set input length overflow")?;
    if n > u32::MAX as usize {
        return Err("Set index range exceeded".into());
    }
    // Both permutations and the existing sort's bounded stable-run allocation.
    budget(n.checked_mul(12))?;
    let mut indices = allocate::<u32>(n, 0, cancel)?;
    let mut scratch = allocate::<u32>(n, bytes(&indices), cancel)?;
    budget(
        bytes(&indices)
            .checked_add(bytes(&scratch))
            .and_then(|b| b.checked_add(4 * n)),
    )?;
    for (i, index) in indices.iter_mut().enumerate() {
        if i.is_multiple_of(CHUNK) {
            checkpoint(cancel, Phase::Initialize)?;
        }
        *index = i as u32;
    }
    checkpoint(cancel, Phase::Sort)?;
    sort_lane(
        &mut indices,
        &mut scratch,
        false,
        &|a, b| inputs.cmp_items(a as usize, b as usize),
        cancel,
    )?;
    checkpoint(cancel, Phase::Sort)?;
    drop(scratch);
    let mut lengths = [0usize; 3];
    scan(&indices, inputs, request, Phase::Count, cancel, |r| {
        lengths[0] += 1;
        lengths[1] += usize::from(r.a != ABSENT);
        lengths[2] += usize::from(r.b != ABSENT);
    })?;
    let (logical, typed) = output_class(inputs, request.operation)?;
    let out_bytes = output_admission(
        lengths,
        inputs.rows_mode,
        inputs.cols,
        request,
        logical,
        typed,
    )?;
    let k = lengths[0];
    // Staged capacities: no result payload before exact output-unit admission.
    budget(bytes(&indices).checked_add(8 * k))?;
    let mut records = allocate::<Record>(k, bytes(&indices), cancel)?;
    let mut next = 0;
    scan(&indices, inputs, request, Phase::Select, cancel, |r| {
        records[next] = r;
        next += 1;
    })?;
    drop(indices);
    let mut order = None;
    if request.order == UniqueOrder::Stable {
        budget(bytes(&records).checked_add(12 * k))?;
        let mut permutation = allocate::<u32>(k, bytes(&records), cancel)?;
        let mut scratch = allocate::<u32>(k, bytes(&records) + bytes(&permutation), cancel)?;
        budget(
            bytes(&records)
                .checked_add(bytes(&permutation))
                .and_then(|b| b.checked_add(bytes(&scratch)))
                .and_then(|b| b.checked_add(4 * k)),
        )?;
        for (i, p) in permutation.iter_mut().enumerate() {
            if i.is_multiple_of(CHUNK) {
                checkpoint(cancel, Phase::Reorder)?;
            }
            *p = i as u32;
        }
        checkpoint(cancel, Phase::Reorder)?;
        sort_lane(
            &mut permutation,
            &mut scratch,
            false,
            &|a, b| {
                inputs
                    .ordinal(records[a as usize])
                    .cmp(&inputs.ordinal(records[b as usize]))
            },
            cancel,
        )?;
        checkpoint(cancel, Phase::Reorder)?;
        drop(scratch);
        order = Some(permutation);
    }
    let live = bytes(&records) + order.as_ref().map_or(0, bytes);
    budget(live.checked_add(out_bytes))?;
    finish(inputs, &records, order.as_deref(), live, request, cancel)
}
impl Array {
    /// Real/logical vector set operations; indices are first-occurrence columns.
    /// Empty orientation and first signed-zero bits are MatRust choices pending
    /// authentic reference checks. NaNs remain distinct and never match.
    pub fn set_operation(
        &self,
        b: &Array,
        operation: SetOperation,
        order: UniqueOrder,
        outputs: [bool; 3],
        cancel: &AtomicBool,
    ) -> Result<SetResult> {
        self.set_operation_bounded(b, operation, order, outputs, usize::MAX, cancel)
    }
    pub(crate) fn set_operation_bounded(
        &self,
        b: &Array,
        operation: SetOperation,
        order: UniqueOrder,
        outputs: [bool; 3],
        units: usize,
        cancel: &AtomicBool,
    ) -> Result<SetResult> {
        self.set_operation_rows_bounded(b, operation, order, false, outputs, units, cancel)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn set_operation_rows_bounded(
        &self,
        b: &Array,
        operation: SetOperation,
        order: UniqueOrder,
        rows_mode: bool,
        outputs: [bool; 3],
        units: usize,
        cancel: &AtomicBool,
    ) -> Result<SetResult> {
        check(cancel)?;
        if operation == SetOperation::Difference && outputs[2] {
            return Err("setdiff supports only C and ia outputs".into());
        }
        if rows_mode {
            if self.is_nd() || b.is_nd() {
                return Err("Set operations with 'rows' are not supported for N-D arrays".into());
            }
            if self.cols() != b.cols() {
                return Err("A and B must have the same number of columns.".into());
            }
        }
        let inputs = Inputs {
            a: admit(self)?,
            b: admit(b)?,
            na: if rows_mode { self.rows() } else { self.len() },
            nb: if rows_mode { b.rows() } else { b.len() },
            row_a: self.rows() == 1,
            row_b: b.rows() == 1,
            rows_mode,
            cols: if rows_mode { self.cols() } else { 1 },
        };
        run(
            &inputs,
            Request {
                operation,
                order,
                outputs,
                units,
            },
            cancel,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, sync::atomic::Ordering};
    thread_local! { static CANCEL: Cell<Option<Phase>> = const { Cell::new(None) }; }
    pub(super) fn probe(cancel: &AtomicBool, phase: Phase) {
        CANCEL.with(|slot| {
            if slot.get() == Some(phase) {
                slot.set(None);
                cancel.store(true, Ordering::Relaxed);
            }
        });
    }
    #[test]
    fn each_phase_cancels_without_publishing_or_changing_aliases_and_retry_works() {
        let a: Array = Matrix::from_column_vec(
            1,
            10001,
            (0..10001).map(|i| ((i * 37) % 997) as f64).collect(),
        )
        .unwrap()
        .into();
        let b: Array = Matrix::from_column_slice(1, 3, &[500., 501., 1001.])
            .unwrap()
            .into();
        let before = (a.clone(), b.clone());
        let cancel = AtomicBool::new(false);
        for phase in [
            Phase::Initialize,
            Phase::Sort,
            Phase::Count,
            Phase::Select,
            Phase::Reorder,
            Phase::Output,
        ] {
            CANCEL.with(|p| p.set(Some(phase)));
            assert!(
                a.set_operation(
                    &b,
                    SetOperation::Union,
                    UniqueOrder::Stable,
                    [true; 3],
                    &cancel
                )
                .unwrap_err()
                .contains("cancelled")
            );
            CANCEL.with(|p| p.set(None));
            assert!(a.same_snapshot(&before.0) && b.same_snapshot(&before.1));
            cancel.store(false, Ordering::Relaxed);
            let result = a
                .set_operation(
                    &b,
                    SetOperation::Union,
                    UniqueOrder::Stable,
                    [true; 3],
                    &cancel,
                )
                .unwrap();
            assert_eq!(result.values.unwrap().len(), 998);
        }
        let logical: Array = LogicalMatrix::filled(1, 10001, true, &cancel)
            .unwrap()
            .into();
        for phase in [Phase::Count, Phase::Output] {
            CANCEL.with(|p| p.set(Some(phase)));
            assert!(
                logical
                    .set_operation(
                        &logical,
                        SetOperation::Intersect,
                        UniqueOrder::Sorted,
                        [true; 3],
                        &cancel
                    )
                    .unwrap_err()
                    .contains("cancelled")
            );
            CANCEL.with(|p| p.set(None));
            cancel.store(false, Ordering::Relaxed);
        }
    }
    #[test]
    fn all_masks_exact_counts_dimension_and_private_limits() {
        let a: Array = Matrix::from_column_slice(1, 4, &[3., 1., 3., 2.])
            .unwrap()
            .into();
        let b: Array = Matrix::from_column_slice(1, 3, &[2., 4., 2.])
            .unwrap()
            .into();
        let cancel = AtomicBool::new(false);
        for operation in [
            SetOperation::Union,
            SetOperation::Intersect,
            SetOperation::Difference,
            SetOperation::SymmetricDifference,
        ] {
            for bits in 0..8 {
                let mask = [bits & 1 != 0, bits & 2 != 0, bits & 4 != 0];
                if operation == SetOperation::Difference && mask[2] {
                    continue;
                }
                if bits == 0 {
                    CANCEL.with(|p| p.set(Some(Phase::Sort)));
                }
                let result = a
                    .set_operation(&b, operation, UniqueOrder::Stable, mask, &cancel)
                    .unwrap();
                assert_eq!(
                    [
                        result.values.is_some(),
                        result.first_a.is_some(),
                        result.first_b.is_some()
                    ],
                    mask
                );
                assert!(!cancel.load(Ordering::Relaxed));
                CANCEL.with(|p| p.set(None));
            }
        }
        assert!(budget(Some(PRIVATE_BYTES)).is_ok());
        assert!(budget(Some(PRIVATE_BYTES + 1)).is_err());
        assert!(budget(None).is_err());
        assert!(budget(crate::matrix::MAX_ELEMENTS.checked_mul(24)).is_err());
        let request = Request {
            operation: SetOperation::Intersect,
            order: UniqueOrder::Sorted,
            outputs: [true; 3],
            units: 6,
        };
        assert_eq!(
            output_admission([2, 2, 2], false, 1, request, true, None).unwrap(),
            34
        );
        assert!(
            output_admission(
                [2, 2, 2],
                false,
                1,
                Request {
                    units: 5,
                    ..request
                },
                true,
                None
            )
            .unwrap_err()
            .contains("evaluation budget")
        );
        assert!(
            output_admission(
                [crate::matrix::MAX_ELEMENTS + 1, 0, 0],
                false,
                1,
                Request {
                    units: usize::MAX,
                    ..request
                },
                false,
                None
            )
            .is_err()
        );
        // No output allocations occur before the exact-unit rejection.
        CANCEL.with(|p| p.set(Some(Phase::Output)));
        assert!(
            a.set_operation_bounded(
                &b,
                SetOperation::Union,
                UniqueOrder::Sorted,
                [true; 3],
                7,
                &cancel
            )
            .unwrap_err()
            .contains("evaluation budget")
        );
        assert!(!cancel.load(Ordering::Relaxed));
        CANCEL.with(|p| p.set(None));
    }
}
