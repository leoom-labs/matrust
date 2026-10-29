//! Borrowed vector sets. Serial production, bounded scratch, exact output counts.
use super::sort::{check, compare, sort_lane};
use crate::{
    Array, LogicalMatrix, Matrix, Result,
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
}
impl Values<'_> {
    fn get(self, i: usize) -> f64 {
        match self {
            Self::Real(a) => a[i],
            Self::Logical(a) => f64::from(a[i]),
        }
    }
    fn logical(self) -> bool {
        matches!(self, Self::Logical(_))
    }
}
struct Inputs<'a> {
    a: Values<'a>,
    b: Values<'a>,
    na: usize,
    nb: usize,
    row_a: bool,
    row_b: bool,
}
impl Inputs<'_> {
    fn read(&self, i: usize) -> f64 {
        if i < self.na {
            self.a.get(i)
        } else {
            self.b.get(i - self.na)
        }
    }
    fn value(&self, r: Record) -> f64 {
        if r.a != ABSENT {
            self.a.get(r.a as usize)
        } else {
            self.b.get(r.b as usize)
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
    if !a.is_empty() && a.rows() != 1 && a.cols() != 1 {
        return Err(
            "Set operations support vectors and shaped empties, not nonempty matrices".into(),
        );
    }
    match a {
        Array::Numeric(a) => Ok(Values::Real(a.data())),
        Array::Logical(a) => Ok(Values::Logical(a.data())),
        _ => Err("Set operations support real double or logical inputs only".into()),
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
fn output_admission(counts: [usize; 3], request: Request, logical: bool) -> Result<usize> {
    let mut units = 0usize;
    let mut size = 0usize;
    for (i, &n) in counts.iter().enumerate() {
        if request.outputs[i] {
            Matrix::checked_len(n, 1)?;
            units = units.checked_add(n).ok_or("Set output count overflow")?;
            size = size
                .checked_add(
                    n.checked_mul(if i == 0 && logical { 1 } else { 8 })
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
        let value = inputs.read(first);
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
            if end == indices.len() || inputs.read(indices[end] as usize) != value {
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
    let logical =
        inputs.a.logical() && (request.operation == SetOperation::Difference || inputs.b.logical());
    let output_bytes = output_admission(lengths, request, logical)?;
    budget(live.checked_add(output_bytes))?;
    checkpoint(cancel, Phase::Output)?;
    let mut real = None;
    let mut bits = None;
    let mut ia = None;
    let mut ib = None;
    if request.outputs[0] {
        if logical {
            let a = allocate::<u8>(lengths[0], live, cancel)?;
            live += bytes(&a);
            bits = Some(a);
        } else {
            let a = allocate::<f64>(lengths[0], live, cancel)?;
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
    for i in 0..records.len() {
        if i.is_multiple_of(CHUNK) {
            checkpoint(cancel, Phase::Output)?;
        }
        let r = records[permutation.map_or(i, |p| p[i] as usize)];
        if let Some(a) = &mut real {
            a[i] = inputs.value(r);
        }
        if let Some(a) = &mut bits {
            a[i] = u8::from(inputs.value(r) != 0.);
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
    let row = inputs.row_a && (request.operation == SetOperation::Difference || inputs.row_b);
    let (rows, cols) = if row {
        (1, lengths[0])
    } else {
        (lengths[0], 1)
    };
    let values = if let Some(a) = real {
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
    if inputs.a.logical() && inputs.b.logical() {
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
        &|a, b| {
            compare(
                inputs.read(a as usize),
                inputs.read(b as usize),
                SortOrder::Ascend,
                MissingPlacement::Last,
            )
        },
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
    let logical = inputs.a.logical() && request.operation == SetOperation::Difference;
    let out_bytes = output_admission(lengths, request, logical)?;
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
        check(cancel)?;
        if operation == SetOperation::Difference && outputs[2] {
            return Err("setdiff supports only C and ia outputs".into());
        }
        let inputs = Inputs {
            a: admit(self)?,
            b: admit(b)?,
            na: self.len(),
            nb: b.len(),
            row_a: self.rows() == 1,
            row_b: b.rows() == 1,
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
        assert_eq!(output_admission([2, 2, 2], request, true).unwrap(), 34);
        assert!(
            output_admission(
                [2, 2, 2],
                Request {
                    units: 5,
                    ..request
                },
                true
            )
            .unwrap_err()
            .contains("evaluation budget")
        );
        assert!(
            output_admission(
                [crate::matrix::MAX_ELEMENTS + 1, 0, 0],
                Request {
                    units: usize::MAX,
                    ..request
                },
                false
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
