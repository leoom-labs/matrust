//! Sparse num2cell builds full cells directly from compressed columns; dense
//! two-input num2cell gathers each block along the source's shape strides.
use super::{Engine, Value};
use crate::{
    Array, CellArray, ComplexMatrix, LogicalMatrix, Matrix, Result, TypedArray,
    containers::{SLOT, checked_totals},
    script_containers::MAX_CELL_SLOTS,
    sparse::{SparseMatrix, SparseValues},
    typed_array::TypedData,
};
use std::{collections::BinaryHeap, ops::Range, sync::Arc};

const CHUNK: usize = 1024;
const INVALID_AXIS: &str = "num2cell: dimensions must be a nonempty vector of positive integers";
const OVERFLOW: &str = "num2cell: storage size overflow";
const ASCENDING: &str =
    "num2cell: dimensions must be listed in ascending order; permuted blocks are not supported yet";
const RANGE: &str = "num2cell: block position is outside the source array";

struct Groups {
    count: usize,
    block_len: usize,
    row_bins: bool,
    group_cols: bool,
}

pub(in crate::engine) fn sparse_num2cell(engine: &Engine, args: &[Value]) -> Result<Value> {
    let [Value::Container(Array::Sparse(source)), rest @ ..] = args else {
        unreachable!("sparse dispatch checks the first argument");
    };
    let (group_rows, group_cols) = match rest {
        [] => (false, false),
        [dim] => {
            let axes = dimensions(engine, dim, 2)?;
            // Same rule as full arrays (Octave 10.3 rejects [2 1]).
            if !axes.ascending {
                return Err(ASCENDING.into());
            }
            (axes.mask & 1 != 0, axes.mask & 2 != 0)
        }
        _ => return Err("num2cell: expected one or two inputs".into()),
    };
    let (rows, cols) = (source.rows(), source.cols());
    let cell_rows = if group_rows { 1 } else { rows };
    let cell_cols = if group_cols { 1 } else { cols };
    let count = cell_rows.checked_mul(cell_cols).ok_or(OVERFLOW)?;
    if cell_rows > MAX_CELL_SLOTS || cell_cols > MAX_CELL_SLOTS || count > MAX_CELL_SLOTS {
        return Err("num2cell: result exceeds 65536 cells or cell dimension bound".into());
    }
    let block_rows = if group_rows { rows } else { 1 };
    let block_cols = if group_cols { cols } else { 1 };
    let block_len = if count == 0 {
        0
    } else {
        Matrix::checked_len(block_rows, block_cols).map_err(|e| format!("num2cell: {e}"))?
    };
    let groups = Groups {
        count,
        block_len,
        row_bins: !group_rows && group_cols && count > 1 && block_len != 0,
        group_cols,
    };
    preflight(engine, &groups, source.is_logical())?;
    engine.check_cancel()?;
    let mut slots = reserved(count)?;
    if count != 0 {
        if !group_rows && !group_cols {
            scalar_slots(engine, source, &mut slots)?;
        } else {
            let stop = || engine.check_cancel();
            match source.values() {
                SparseValues::Double(values) => grouped_slots(
                    source,
                    &mut slots,
                    &groups,
                    &stop,
                    |k| values[k],
                    |data| Ok(Matrix::from_column_vec(block_rows, block_cols, data)?.into()),
                )?,
                SparseValues::Logical(values) => grouped_slots(
                    source,
                    &mut slots,
                    &groups,
                    &stop,
                    |k| u8::from(values[k]),
                    |data| {
                        Ok(LogicalMatrix::from_column_vec(
                            block_rows,
                            block_cols,
                            data,
                            &engine.cancel,
                        )?
                        .into())
                    },
                )?,
            }
        }
    }
    Ok(CellArray::from_shared(cell_rows, cell_cols, slots, &engine.cancel)?.into_array_value())
}

/// Validated grouping axes. `mask` holds the zero-based axes below the source
/// rank; axes above it are singleton and group nothing.
struct Axes {
    mask: u64,
    ascending: bool,
}

fn dimensions(engine: &Engine, dim: &Value, rank: usize) -> Result<Axes> {
    let (rows, cols) = match dim {
        Value::Matrix(a) if !a.is_nd() => (a.rows(), a.cols()),
        Value::Logical(a) if !a.is_nd() => (a.rows(), a.cols()),
        Value::Container(Array::Typed(a)) if !a.is_nd() => (a.rows(), a.cols()),
        Value::Container(Array::Sparse(a)) if a.nnz() as u64 == a.numel() => (a.rows(), a.cols()),
        _ => return Err(INVALID_AXIS.into()),
    };
    let len = rows.checked_mul(cols).ok_or(OVERFLOW)?;
    if len == 0 || (rows != 1 && cols != 1) {
        return Err(INVALID_AXIS.into());
    }
    budget(engine, len.checked_mul(size_of::<u64>()).ok_or(OVERFLOW)?)?;
    engine.check_cancel()?;
    // Insert and remove incrementally so there is one bounded buffer and a
    // Stop checkpoint during both key collection and duplicate checking.
    let mut keys = BinaryHeap::new();
    keys.try_reserve_exact(len)
        .map_err(|_| "num2cell: unable to allocate dimension keys")?;
    let mut axes = Axes {
        mask: 0,
        ascending: true,
    };
    let mut last = 0;
    for i in 0..len {
        if i.is_multiple_of(CHUNK) {
            engine.check_cancel()?;
        }
        let (key, axis) = match dim {
            Value::Matrix(a) => float_axis(a.data()[i], rank)?,
            Value::Logical(a) => float_axis(f64::from(a.data()[i]), rank)?,
            Value::Container(Array::Sparse(a)) => float_axis(a.value(i), rank)?,
            Value::Container(Array::Typed(a)) if a.is_integer() => {
                let n = u64::try_from(a.int(i)).map_err(|_| INVALID_AXIS)?;
                if n == 0 {
                    return Err(INVALID_AXIS.into());
                }
                let axis = usize::try_from(n).ok().filter(|&n| n <= rank);
                (n, axis.map(|n| n - 1))
            }
            Value::Container(Array::Typed(a)) => float_axis(a.real(i), rank)?,
            _ => unreachable!("checked dimension shape"),
        };
        // The keys of one array share a representation ordered like its values.
        axes.ascending &= key > last;
        last = key;
        keys.push(key);
        if let Some(axis) = axis {
            axes.mask |= 1 << axis;
        }
    }
    let mut previous = 0;
    for (i, key) in std::iter::from_fn(|| keys.pop()).enumerate() {
        if i.is_multiple_of(CHUNK) {
            engine.check_cancel()?;
        }
        if key == previous {
            return Err("num2cell: dimensions must be distinct".into());
        }
        previous = key;
    }
    engine.check_cancel()?;
    Ok(axes)
}

fn float_axis(n: f64, rank: usize) -> Result<(u64, Option<usize>)> {
    if n >= 1. && n.fract() == 0. {
        // Within a floating-point array equal positive integers have equal bits,
        // including whole values too large for u64.
        Ok((n.to_bits(), (n <= rank as f64).then(|| n as usize - 1)))
    } else {
        Err(INVALID_AXIS.into())
    }
}

fn budget(engine: &Engine, bytes: usize) -> Result<()> {
    if bytes > super::super::limits::memory_bytes(&engine.limits) {
        return Err(super::super::limits::memory_error(
            "num2cell",
            &engine.limits,
        ));
    }
    Ok(())
}

fn preflight(engine: &Engine, groups: &Groups, logical: bool) -> Result<()> {
    let count = groups.count;
    let (element, backing) = if logical {
        (size_of::<u8>(), size_of::<Vec<u8>>())
    } else {
        (size_of::<f64>(), size_of::<nalgebra::DMatrix<f64>>())
    };
    let payload = count
        .checked_mul(groups.block_len)
        .and_then(|n| n.checked_mul(element))
        .ok_or(OVERFLOW)?;
    let cell_bytes = count
        .checked_mul(SLOT)
        .and_then(|n| n.checked_add(payload))
        .ok_or(OVERFLOW)?;
    checked_totals(cell_bytes, count, 1)?;
    // Slots and their Array objects, two Arc headers per block, the backing
    // objects, and the shared slot vector allocation. Row buffers become the
    // final block payloads, but their Vec headers stay live as slots are built.
    let arc_header = 2 * size_of::<usize>();
    let bin = if groups.row_bins {
        size_of::<Vec<f64>>()
    } else {
        0
    };
    let peak = count
        .checked_mul(2 * arc_header + backing + bin)
        .and_then(|n| n.checked_add(cell_bytes))
        .and_then(|n| n.checked_add(arc_header + size_of::<Vec<Arc<Array>>>()))
        .ok_or(OVERFLOW)?;
    budget(engine, peak)
}

fn reserved<T>(len: usize) -> Result<Vec<T>> {
    let mut data = Vec::new();
    data.try_reserve_exact(len)
        .map_err(|_| "num2cell: unable to allocate cell buffers")?;
    Ok(data)
}

fn zeroed<T: Copy + Default>(len: usize, stop: &impl Fn() -> Result<()>) -> Result<Vec<T>> {
    stop()?;
    let mut data = reserved(len)?;
    while data.len() < len {
        stop()?;
        data.resize((data.len() + CHUNK).min(len), T::default());
    }
    Ok(data)
}

fn scalar_slots(engine: &Engine, source: &SparseMatrix, slots: &mut Vec<Arc<Array>>) -> Result<()> {
    let logical = source.is_logical();
    for c in 0..source.cols() {
        let mut entries = source.column(c).peekable();
        for r in 0..source.rows() {
            if slots.len().is_multiple_of(CHUNK) {
                engine.check_cancel()?;
            }
            let v = if entries.peek().is_some_and(|&k| source.rowidx()[k] == r) {
                source.value(entries.next().unwrap())
            } else {
                0.
            };
            let item = if logical {
                LogicalMatrix::scalar(v != 0.).into()
            } else {
                Matrix::scalar(v).into()
            };
            slots.push(Arc::new(item));
        }
    }
    Ok(())
}

fn grouped_slots<T: Copy + Default>(
    source: &SparseMatrix,
    slots: &mut Vec<Arc<Array>>,
    groups: &Groups,
    stop: &impl Fn() -> Result<()>,
    read: impl Fn(usize) -> T,
    wrap: impl Fn(Vec<T>) -> Result<Array>,
) -> Result<()> {
    if groups.row_bins {
        let mut rows = reserved(groups.count)?;
        for _ in 0..groups.count {
            rows.push(zeroed(groups.block_len, stop)?);
        }
        visit(source, 0..source.cols(), stop, |r, c, k| {
            rows[r][c] = read(k);
        })?;
        for row in rows {
            stop()?;
            slots.push(Arc::new(wrap(row)?));
        }
    } else {
        for i in 0..groups.count {
            let mut data = zeroed(groups.block_len, stop)?;
            if groups.block_len != 0 {
                let columns = if groups.group_cols {
                    0..source.cols()
                } else {
                    i..i + 1
                };
                visit(source, columns, stop, |r, c, k| {
                    data[c * source.rows() + r] = read(k);
                })?;
            }
            slots.push(Arc::new(wrap(data)?));
        }
    }
    Ok(())
}

fn visit(
    source: &SparseMatrix,
    columns: Range<usize>,
    stop: &impl Fn() -> Result<()>,
    mut entry: impl FnMut(usize, usize, usize),
) -> Result<()> {
    for (offset, c) in columns.enumerate() {
        if offset.is_multiple_of(CHUNK) {
            stop()?;
        }
        for k in source.column(c) {
            if k.is_multiple_of(CHUNK) {
                stop()?;
            }
            entry(source.rowidx()[k], offset, k);
        }
    }
    stop()
}

/// A dense source borrowed in its own class: blocks are gathered from this
/// storage directly, with no whole-input cast or copy.
enum Dense<'a> {
    Double(&'a Matrix),
    Logical(&'a LogicalMatrix),
    Complex(&'a ComplexMatrix),
    Typed(&'a TypedArray),
}

impl<'a> Dense<'a> {
    fn of(value: &'a Value) -> Result<Self> {
        Ok(match value {
            Value::Matrix(a) | Value::Container(Array::Numeric(a)) => Self::Double(a),
            Value::Logical(a) | Value::Container(Array::Logical(a)) => Self::Logical(a),
            Value::Complex(a) | Value::Container(Array::Complex(a)) => Self::Complex(a),
            Value::Container(Array::Typed(a)) => Self::Typed(a),
            _ => return Err("num2cell: only numeric and logical inputs are supported yet".into()),
        })
    }
    fn dims(&self) -> std::borrow::Cow<'a, [usize]> {
        match self {
            Self::Double(a) => a.dims(),
            Self::Logical(a) => a.dims(),
            Self::Complex(a) => a.dims(),
            Self::Typed(a) => a.dims(),
        }
    }
    /// Payload bytes of one element and the fixed backing object of one block.
    fn layout(&self) -> (usize, usize) {
        match self {
            Self::Double(_) => (size_of::<f64>(), size_of::<nalgebra::DMatrix<f64>>()),
            Self::Logical(_) => (size_of::<u8>(), size_of::<Vec<u8>>()),
            Self::Complex(_) => (
                size_of::<crate::Complex64>(),
                size_of::<nalgebra::DMatrix<crate::Complex64>>(),
            ),
            Self::Typed(a) => (a.class().size(), size_of::<Vec<u8>>()),
        }
    }
    /// The source itself as a cell element, sharing its immutable storage.
    fn shared(&self) -> Array {
        match *self {
            Self::Double(a) => Array::Numeric(a.clone()),
            Self::Logical(a) => Array::Logical(a.clone()),
            Self::Complex(a) => Array::Complex(a.clone()),
            Self::Typed(a) => Array::Typed(a.clone()),
        }
    }
}

/// One stepped source axis: an odometer digit and its column-major stride.
struct Axis {
    extent: usize,
    stride: usize,
    at: usize,
}

/// Block order and positions. The leading grouped axes are contiguous in
/// column-major storage and copy as one `run`; `inner` steps the remaining
/// grouped axes within a block and `outer` the ungrouped axes between blocks.
struct Walk {
    count: usize,
    block_len: usize,
    run: usize,
    inner: Vec<Axis>,
    outer: Vec<Axis>,
}

struct DensePlan {
    cell: (usize, usize),
    /// Folded block shape, and its dimensions when a block is N-D.
    block: (usize, usize),
    block_dims: Vec<usize>,
    walk: Walk,
}

/// `num2cell(A, dims)` for full numeric, logical, complex and typed arrays.
/// Every listed dimension stays inside a block and every other one indexes
/// the outer cell grid, which must be two-dimensional.
pub(in crate::engine) fn dense_num2cell(
    engine: &Engine,
    source: &Value,
    dim: &Value,
) -> Result<Value> {
    let source = Dense::of(source)?;
    let dims = source.dims();
    let axes = dimensions(engine, dim, dims.len())?;
    if !axes.ascending {
        return Err(ASCENDING.into());
    }
    let DensePlan {
        cell,
        block: (rows, cols),
        block_dims,
        mut walk,
    } = dense_plan(engine, &dims, axes.mask, source.layout())?;
    engine.check_cancel()?;
    let mut slots = reserved(walk.count)?;
    if walk.count == 1 {
        // Every ungrouped extent is 1, so the one block has the source's own
        // dimensions and elements: share its storage instead of copying it.
        slots.push(Arc::new(source.shared()));
    } else {
        let cancel = &engine.cancel;
        let stop = || engine.check_cancel();
        macro_rules! blocks {
            ($values:expr, $variant:path, $data:ident => $build:expr) => {
                dense_blocks($values, &mut walk, CHUNK, &stop, |$data| {
                    let block = $build?;
                    let block = if block_dims.is_empty() {
                        block
                    } else {
                        block.with_dims(&block_dims)?
                    };
                    slots.push(Arc::new($variant(block)));
                    Ok(())
                })?
            };
        }
        macro_rules! typed_blocks {
            ($array:expr, $($class:ident),+) => {
                match $array.data() {
                    $(TypedData::$class(values) => blocks!(
                        values.as_slice(),
                        Array::Typed,
                        data => TypedArray::checked(rows, cols, TypedData::$class(Arc::new(data)))
                    ),)+
                }
            };
        }
        match source {
            Dense::Double(a) => blocks!(
                a.data(),
                Array::Numeric,
                data => Matrix::from_column_vec(rows, cols, data)
            ),
            Dense::Logical(a) => blocks!(
                a.data(),
                Array::Logical,
                data => LogicalMatrix::from_column_vec(rows, cols, data, cancel)
            ),
            Dense::Complex(a) => blocks!(
                a.data(),
                Array::Complex,
                data => ComplexMatrix::from_column_vec(rows, cols, data, cancel)
                    .map(|block| block.with_single_class(a.is_single()))
            ),
            Dense::Typed(a) => typed_blocks!(
                a, Int8, Uint8, Int16, Uint16, Int32, Uint32, Int64, Uint64, Single
            ),
        }
    }
    Ok(CellArray::from_shared(cell.0, cell.1, slots, &engine.cancel)?.into_array_value())
}

/// Shapes, bounds and the memory preflight; nothing is allocated before the
/// budget admits the cell slots, every retained block and the walk's scratch.
fn dense_plan(
    engine: &Engine,
    dims: &[usize],
    mask: u64,
    (element, backing): (usize, usize),
) -> Result<DensePlan> {
    let rank = dims.len();
    let grouped = |k: usize| (mask >> k) & 1 == 1;
    // Extent of axis `k` in a block (`true`) or in the outer cell grid.
    let side = |k: usize, block: bool| if grouped(k) == block { dims[k] } else { 1 };
    let shape_rank = |block: bool| {
        (2..rank)
            .rev()
            .find(|&k| side(k, block) != 1)
            .map_or(2, |k| k + 1)
    };
    // An ungrouped extent other than 1 beyond the second axis is a genuine
    // N-D cell shape. Refuse it by name; folding it would misplace blocks.
    let grid_rank = shape_rank(false);
    if grid_rank > 2 {
        let text = |n: usize, block: Option<bool>| {
            let extents = (0..n).map(|k| block.map_or(dims[k], |b| side(k, b)).to_string());
            extents.collect::<Vec<_>>().join("x")
        };
        return Err(format!(
            "num2cell: the ungrouped dimensions of a {} array form a {} cell grid. {}",
            text(rank, None),
            text(grid_rank, Some(false)),
            super::super::nd::CELL_ND
        ));
    }
    let cell = (side(0, false), side(1, false));
    let count = cell.0.checked_mul(cell.1).ok_or(OVERFLOW)?;
    if cell.0 > MAX_CELL_SLOTS || cell.1 > MAX_CELL_SLOTS || count > MAX_CELL_SLOTS {
        return Err("num2cell: result exceeds 65536 cells or cell dimension bound".into());
    }
    let block_rank = shape_rank(true);
    let lead = (0..rank).take_while(|&k| grouped(k)).count();
    let inner = (lead..rank).filter(|&k| grouped(k)).count();
    let outer = rank - lead - inner;
    let mut block = (side(0, true), 1usize);
    let (mut block_len, mut run) = (0, 1usize);
    if count != 0 {
        for k in 1..rank {
            block.1 = block.1.checked_mul(side(k, true)).ok_or(OVERFLOW)?;
        }
        block_len = block.0.checked_mul(block.1).ok_or(OVERFLOW)?;
    }
    // Strides matter only when an element is read; a nonempty source bounds
    // every partial product of its dimensions by its element count.
    let live = block_len != 0;
    if live {
        for &extent in &dims[..lead] {
            run = run.checked_mul(extent).ok_or(OVERFLOW)?;
        }
    }
    let header = 2 * size_of::<usize>();
    let shape_len = if block_rank > 2 { block_rank } else { 0 };
    // An N-D block retains its own shared dimension vector.
    let shape = if shape_len == 0 {
        0
    } else {
        header + size_of::<Vec<usize>>() + shape_len * size_of::<usize>()
    };
    let payload = count
        .checked_mul(block_len)
        .and_then(|n| n.checked_mul(element))
        .ok_or(OVERFLOW)?;
    let cell_bytes = count
        .checked_mul(SLOT)
        .and_then(|n| n.checked_add(payload))
        .ok_or(OVERFLOW)?;
    checked_totals(cell_bytes, count, 1)?;
    // Slots and their Array objects, two Arc headers and the backing object
    // per block, the shared slot vector, and the scratch live while blocks are
    // gathered: one odometer digit per stepped axis and the block dimensions.
    let scratch = (inner + outer) * size_of::<Axis>() + shape_len * size_of::<usize>();
    let peak = count
        .checked_mul(2 * header + backing + shape)
        .and_then(|n| n.checked_add(cell_bytes))
        .and_then(|n| n.checked_add(header + size_of::<Vec<Arc<Array>>>() + scratch))
        .ok_or(OVERFLOW)?;
    budget(engine, peak)?;
    engine.check_cancel()?;
    let mut plan = DensePlan {
        cell,
        block,
        block_dims: reserved(shape_len)?,
        walk: Walk {
            count,
            block_len,
            run,
            inner: reserved(inner)?,
            outer: reserved(outer)?,
        },
    };
    let mut stride = 1usize;
    for (k, &extent) in dims.iter().enumerate() {
        if k < shape_len {
            plan.block_dims.push(side(k, true));
        }
        let axis = Axis {
            extent,
            stride: if live { stride } else { 0 },
            at: 0,
        };
        if !grouped(k) {
            plan.walk.outer.push(axis);
        } else if k >= lead {
            plan.walk.inner.push(axis);
        }
        stride = stride.saturating_mul(extent);
    }
    Ok(plan)
}

/// Gathers every block in column-major cell order. `stop` runs when a block
/// starts and again after every `chunk` gathered elements, so cancellation is
/// observed inside a block and not only between blocks.
fn dense_blocks<T: Copy>(
    source: &[T],
    walk: &mut Walk,
    chunk: usize,
    stop: &impl Fn() -> Result<()>,
    mut emit: impl FnMut(Vec<T>) -> Result<()>,
) -> Result<()> {
    let chunk = chunk.max(1);
    let mut base = 0usize;
    for _ in 0..walk.count {
        stop()?;
        let mut data = reserved(walk.block_len)?;
        let (mut offset, mut allowance) = (base, chunk);
        while data.len() < walk.block_len {
            let end = offset.checked_add(walk.run).ok_or(RANGE)?;
            let mut start = offset;
            while start < end {
                if allowance == 0 {
                    stop()?;
                    allowance = chunk;
                }
                let n = (end - start).min(allowance);
                data.extend_from_slice(source.get(start..start + n).ok_or(RANGE)?);
                start += n;
                allowance -= n;
            }
            offset = step(&mut walk.inner, offset);
        }
        emit(data)?;
        base = step(&mut walk.outer, base);
    }
    Ok(())
}

/// Advances a column-major odometer and returns the source offset it moved to.
/// A full cycle returns every digit and the offset to their start.
fn step(axes: &mut [Axis], mut offset: usize) -> usize {
    for axis in axes {
        axis.at += 1;
        offset += axis.stride;
        if axis.at < axis.extent {
            break;
        }
        offset -= axis.at * axis.stride;
        axis.at = 0;
    }
    offset
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, sync::atomic::Ordering};

    #[test]
    fn num2cell_row_bins_count_toward_peak() {
        let mut engine = Engine::default();
        engine.execute("a=sparse([1 3],[1 2],[1 1],3,2);").unwrap();
        let mut limits = engine.limits_active();
        let headers = 2 * size_of::<usize>();
        let output = size_of::<Vec<Arc<Array>>>()
            + headers
            + 3 * (SLOT + 2 * headers + size_of::<nalgebra::DMatrix<f64>>())
            + 6 * size_of::<f64>();
        let peak = output + 3 * size_of::<Vec<f64>>();
        for bytes in [output, peak - 1] {
            limits.memory_bytes = bytes;
            engine.limits_set(limits);
            let message = engine.execute("c=num2cell(a,2);").unwrap_err();
            assert!(message.contains("num2cell exceeds"), "{message}");
            assert!(engine.array("c").is_none());
        }
        limits.memory_bytes = peak;
        engine.limits_set(limits);
        engine.execute("c=num2cell(a,2);").unwrap();
        assert!(engine.array("c").is_some());
    }

    #[test]
    fn num2cell_stops_inside_one_stored_column() {
        let engine = Engine::default();
        let rows = CHUNK + 1;
        let source = SparseMatrix::from_parts(
            rows,
            1,
            vec![0, rows],
            (0..rows).collect(),
            SparseValues::Logical(vec![true; rows]),
        )
        .unwrap();
        let visited = Cell::new(0);
        let stop = || {
            if visited.get() == CHUNK {
                engine.cancel.store(true, Ordering::Relaxed);
            }
            engine.check_cancel()
        };
        let result = visit(&source, 0..1, &stop, |_, _, _| {
            visited.set(visited.get() + 1);
        });
        assert_eq!(result.unwrap_err(), "Calculation cancelled");
        assert_eq!(visited.get(), CHUNK);
    }

    type Block = (Vec<usize>, &'static str, Vec<f64>);

    /// Outer cell shape and every block as (dimensions, class, real values).
    fn dense_cells(engine: &Engine, name: &str) -> ((usize, usize), Vec<Block>) {
        let Some(Array::Cell(cells)) = engine.array(name) else {
            panic!("{name} is not a cell array");
        };
        let items = (0..cells.len()).map(|i| {
            let item = cells.get_linear(i).unwrap();
            let values = match item.as_ref() {
                Array::Numeric(m) => m.data().to_vec(),
                Array::Logical(m) => m.data().iter().map(|&v| f64::from(v)).collect(),
                Array::Typed(m) => (0..m.len()).map(|k| m.real(k)).collect(),
                other => panic!("{name} slot {i} is not a real block: {other:?}"),
            };
            (item.dims().to_vec(), item.class(), values)
        });
        ((cells.rows(), cells.cols()), items.collect())
    }

    fn doubles(dims: &[usize], values: &[f64]) -> Block {
        (dims.to_vec(), "double", values.to_vec())
    }

    // GNU Octave 10.3 controls: A=reshape(1:6,2,3), num2cell(A,1) is 1x3 with
    // C{2}=[3;4]; B=reshape(1:8,2,2,2), num2cell(B,[1 3]) is 1x2 of 2x1x2 blocks
    // [1;2;5;6] and [3;4;7;8]; num2cell(B,[2 3]) is 2x1 of 1x2x2 blocks with
    // C{2}(:)' = 2 4 6 8; num2cell(B,[1 2 3]) is 1x1.
    #[test]
    fn corpus_dense_num2cell_matches_octave_controls() {
        let mut engine = Engine::default();
        engine
            .execute(
                "a=reshape(1:6,2,3); c=num2cell(a,1); n=numel(c); x=c{2};
                 r=num2cell(a,2); nr=numel(num2cell(a,2));
                 b=reshape(1:8,2,2,2); d=num2cell(b,[1 3]); e=num2cell(b,[2 3]);
                 w=num2cell(b,[1 2 3]); v=num2cell(b,3);",
            )
            .unwrap();
        let columns = [[1., 2.], [3., 4.], [5., 6.]].map(|v| doubles(&[2, 1], &v));
        assert_eq!(dense_cells(&engine, "c"), ((1, 3), columns.to_vec()));
        let scalar = |name: &str| match engine.array(name) {
            Some(Array::Numeric(m)) => m.data().to_vec(),
            other => panic!("{name}: {other:?}"),
        };
        assert_eq!(scalar("n"), [3.]);
        assert_eq!(scalar("nr"), [2.]);
        assert_eq!(scalar("x"), [3., 4.]);
        let rows = [[1., 3., 5.], [2., 4., 6.]].map(|v| doubles(&[1, 3], &v));
        assert_eq!(dense_cells(&engine, "r"), ((2, 1), rows.to_vec()));
        let d = [[1., 2., 5., 6.], [3., 4., 7., 8.]].map(|v| doubles(&[2, 1, 2], &v));
        assert_eq!(dense_cells(&engine, "d"), ((1, 2), d.to_vec()));
        let e = [[1., 3., 5., 7.], [2., 4., 6., 8.]].map(|v| doubles(&[1, 2, 2], &v));
        assert_eq!(dense_cells(&engine, "e"), ((2, 1), e.to_vec()));
        let whole = doubles(&[2, 2, 2], &[1., 2., 3., 4., 5., 6., 7., 8.]);
        assert_eq!(dense_cells(&engine, "w"), ((1, 1), vec![whole]));
        // Grouping only the third axis leaves a representable 2x2 grid.
        let pages = [[1., 5.], [2., 6.], [3., 7.], [4., 8.]].map(|v| doubles(&[1, 1, 2], &v));
        assert_eq!(dense_cells(&engine, "v"), ((2, 2), pages.to_vec()));
    }

    #[test]
    fn corpus_dense_num2cell_refuses_an_nd_outer_grid() {
        let mut engine = Engine::default();
        engine
            .execute("b=reshape(1:8,2,2,2); t=reshape(1:4,2,1,2); z=zeros(0,2,2);")
            .unwrap();
        for (call, size, grid) in [
            ("num2cell(b,1)", "2x2x2", "1x2x2"),
            ("num2cell(b,2)", "2x2x2", "2x1x2"),
            ("num2cell(b,5)", "2x2x2", "2x2x2"),
            ("num2cell(t,1)", "2x1x2", "1x1x2"),
            ("num2cell(t,[1 2])", "2x1x2", "1x1x2"),
            // An empty N-D grid is still N-D: no 1x0 or 0x0 stand-in.
            ("num2cell(z,1)", "0x2x2", "1x2x2"),
            ("num2cell(z,2)", "0x2x2", "0x1x2"),
        ] {
            let message = engine.execute(&format!("c={call};")).unwrap_err();
            let limit = format!("of a {size} array form a {grid} cell grid");
            assert!(message.contains(&limit), "{call}: {message}");
            assert!(
                message.contains(super::super::super::nd::CELL_ND),
                "{call}: {message}"
            );
            assert!(engine.array("c").is_none(), "{call}");
        }
        // The shape refusal precedes the memory preflight and any allocation.
        let mut limits = engine.limits_active();
        limits.memory_bytes = size_of::<u64>();
        engine.limits_set(limits);
        let message = engine.execute("c=num2cell(b,1);").unwrap_err();
        assert!(message.contains("1x2x2 cell grid"), "{message}");
        // One input keeps its established refusal of an N-D source.
        let message = engine.execute("c=num2cell(b);").unwrap_err();
        assert!(
            message.contains("num2cell does not support N-D arrays yet (size 2x2x2)"),
            "{message}"
        );
        assert!(engine.array("c").is_none());
    }

    #[test]
    fn corpus_dense_num2cell_keeps_class_and_exact_integers() {
        let mut engine = Engine::default();
        engine
            .execute(
                "h=bitshift(uint64(1),53);
                 u=[h+uint64(1), h+uint64(3); h+uint64(5), h+uint64(7)];
                 cu=num2cell(u,1); ru=num2cell(u,2);
                 m=intmax('int64'); s=reshape([m-7 m-6 m-5 m-4 m-3 m-2 m-1 m],2,2,2);
                 cs=num2cell(s,[1 3]);
                 l=reshape(logical([1 0 0 1 1 1 0 0]),2,2,2); cl=num2cell(l,[2 3]);
                 f=single([1.5 2.5;3.5 4.5]); cf=num2cell(f,1);
                 z=single([1 2;3 4]); z(1,2)=2+3i; cz=num2cell(z,2);
                 y=single(zeros(2,2,2)); y(2,1,2)=0.5+3i; cy=num2cell(y,[1 3]);
                 q=[1+2i 3; 4 5-1i]; cq=num2cell(q,1);",
            )
            .unwrap();
        let cell = |name: &str| match engine.array(name) {
            Some(Array::Cell(cells)) => cells,
            other => panic!("{name}: {other:?}"),
        };
        // Exact 64-bit values above 2^53: adjacent odd integers no double holds.
        let ints = |name: &str, class: crate::NumericClass, dims: &[usize]| {
            let cells = cell(name);
            let blocks = (0..cells.len()).map(|i| match cells.get_linear(i).unwrap().as_ref() {
                Array::Typed(block) => {
                    assert_eq!(
                        (block.class(), &*block.dims()),
                        (class, dims),
                        "{name} slot {i}"
                    );
                    (0..block.len()).map(|k| block.int(k)).collect::<Vec<_>>()
                }
                other => panic!("{name} slot {i}: {other:?}"),
            });
            ((cells.rows(), cells.cols()), blocks.collect::<Vec<_>>())
        };
        let h = 1i128 << 53;
        let unsigned = crate::NumericClass::Uint64;
        assert_eq!(
            ints("cu", unsigned, &[2, 1]),
            ((1, 2), vec![vec![h + 1, h + 5], vec![h + 3, h + 7]])
        );
        assert_eq!(
            ints("ru", unsigned, &[1, 2]),
            ((2, 1), vec![vec![h + 1, h + 3], vec![h + 5, h + 7]])
        );
        let m = i128::from(i64::MAX);
        assert_eq!(
            ints("cs", crate::NumericClass::Int64, &[2, 1, 2]),
            (
                (1, 2),
                vec![
                    vec![m - 7, m - 6, m - 3, m - 2],
                    vec![m - 5, m - 4, m - 1, m]
                ]
            )
        );
        let logical =
            [[1., 0., 1., 0.], [0., 1., 1., 0.]].map(|v| (vec![1, 2, 2], "logical", v.to_vec()));
        assert_eq!(dense_cells(&engine, "cl"), ((2, 1), logical.to_vec()));
        let single = [[1.5, 3.5], [2.5, 4.5]].map(|v| (vec![2, 1], "single", v.to_vec()));
        assert_eq!(dense_cells(&engine, "cf"), ((1, 2), single.to_vec()));
        let complex = |name: &str, single: bool, dims: &[usize]| {
            let cells = cell(name);
            let blocks = (0..cells.len()).map(|i| match cells.get_linear(i).unwrap().as_ref() {
                Array::Complex(block) => {
                    assert_eq!(
                        (block.is_single(), &*block.dims()),
                        (single, dims),
                        "{name} slot {i}"
                    );
                    block
                        .data()
                        .iter()
                        .map(|v| (v.re, v.im))
                        .collect::<Vec<_>>()
                }
                other => panic!("{name} slot {i}: {other:?}"),
            });
            ((cells.rows(), cells.cols()), blocks.collect::<Vec<_>>())
        };
        assert_eq!(
            complex("cz", true, &[1, 2]),
            (
                (2, 1),
                vec![vec![(1., 0.), (2., 3.)], vec![(3., 0.), (4., 0.)]]
            )
        );
        assert_eq!(cell("cz").get_linear(0).unwrap().class(), "single");
        let zero = (0., 0.);
        assert_eq!(
            complex("cy", true, &[2, 1, 2]),
            (
                (1, 2),
                vec![vec![zero, zero, zero, (0.5, 3.)], vec![zero; 4]]
            )
        );
        assert_eq!(
            complex("cq", false, &[2, 1]),
            (
                (1, 2),
                vec![vec![(1., 2.), (4., 0.)], vec![(3., 0.), (5., -1.)]]
            )
        );
        assert_eq!(cell("cq").get_linear(1).unwrap().class(), "double");
    }

    #[test]
    fn corpus_dense_num2cell_validates_axes_apart_from_block_values() {
        let mut engine = Engine::default();
        engine
            .execute(
                "a=reshape(1:6,2,3); h=bitshift(uint64(1),53); far=[h,h+uint64(1)];
                 k=num2cell(a,far); col=num2cell(a,[1,h]); row=num2cell(a,[2 5]);
                 one=num2cell(a,true); all=num2cell(a,sparse([1 2])); tall=num2cell(a,[1;2]);
                 i8=num2cell(a,int8(2)); s1=num2cell(a,single(1));",
            )
            .unwrap();
        // Adjacent uint64 axes above 2^53 are distinct keys, never equal doubles.
        let Some(Array::Typed(far)) = engine.array("far") else {
            panic!("expected exact uint64 axes");
        };
        assert_eq!((far.int(0), far.int(1)), (1 << 53, (1 << 53) + 1));
        let scalars = [1., 2., 3., 4., 5., 6.].map(|v| doubles(&[1, 1], &[v]));
        assert_eq!(dense_cells(&engine, "k"), ((2, 3), scalars.to_vec()));
        let columns = [[1., 2.], [3., 4.], [5., 6.]].map(|v| doubles(&[2, 1], &v));
        for name in ["col", "one", "s1"] {
            assert_eq!(
                dense_cells(&engine, name),
                ((1, 3), columns.to_vec()),
                "{name}"
            );
        }
        let rows = [[1., 3., 5.], [2., 4., 6.]].map(|v| doubles(&[1, 3], &v));
        for name in ["row", "i8"] {
            assert_eq!(
                dense_cells(&engine, name),
                ((2, 1), rows.to_vec()),
                "{name}"
            );
        }
        let whole = doubles(&[2, 3], &[1., 2., 3., 4., 5., 6.]);
        for name in ["all", "tall"] {
            assert_eq!(
                dense_cells(&engine, name),
                ((1, 1), vec![whole.clone()]),
                "{name}"
            );
        }
        // A rejected axis list never reaches the blocks: the same source and
        // values fail on the list alone, with its own message.
        for (dims, reason) in [
            ("[h,h]", "dimensions must be distinct"),
            ("[3 3]", "dimensions must be distinct"),
            ("[1 1]", "dimensions must be distinct"),
            ("[]", "vector of positive integers"),
            ("0", "vector of positive integers"),
            ("-1", "vector of positive integers"),
            ("1.5", "vector of positive integers"),
            ("Inf", "vector of positive integers"),
            ("NaN", "vector of positive integers"),
            ("int8(-1)", "vector of positive integers"),
            ("uint8(0)", "vector of positive integers"),
            ("[1 2;3 4]", "vector of positive integers"),
            ("ones(1,1,2)", "vector of positive integers"),
            ("sparse([1 0])", "vector of positive integers"),
            ("1+2i", "vector of positive integers"),
            ("{1}", "vector of positive integers"),
            ("[2 1]", "must be listed in ascending order"),
            ("[3 1]", "must be listed in ascending order"),
            ("[h+uint64(1),h]", "must be listed in ascending order"),
        ] {
            let message = engine
                .execute(&format!("bad=num2cell(a,{dims});"))
                .unwrap_err();
            assert!(message.contains(reason), "{dims}: {message}");
            assert!(engine.array("bad").is_none(), "{dims}");
        }
        for source in ["{1,2}", "'abc'"] {
            let message = engine
                .execute(&format!("bad=num2cell({source},1);"))
                .unwrap_err();
            assert!(
                message.contains("only numeric and logical inputs"),
                "{source}: {message}"
            );
        }
        let message = engine.execute("bad=num2cell(a,1,2);").unwrap_err();
        assert!(message.contains("expected one or two inputs"), "{message}");
        // An N-D source reports a bad list the same way, and a handle call
        // reaches the same grouping.
        engine
            .execute("b=reshape(1:8,2,2,2); f=@num2cell; d=f(b,[1 3]);")
            .unwrap();
        let d = [[1., 2., 5., 6.], [3., 4., 7., 8.]].map(|v| doubles(&[2, 1, 2], &v));
        assert_eq!(dense_cells(&engine, "d"), ((1, 2), d.to_vec()));
        for dims in ["'x'", "[1 1.5]", "ones(1,1,2)"] {
            let message = engine
                .execute(&format!("bad=num2cell(b,{dims});"))
                .unwrap_err();
            assert!(
                message.contains("vector of positive integers"),
                "{dims}: {message}"
            );
        }
        assert!(engine.array("bad").is_none());
    }

    #[test]
    fn corpus_dense_num2cell_groups_empty_and_singleton_axes() {
        let mut engine = Engine::default();
        engine
            .execute(
                "a=zeros(0,3); ac=num2cell(a,1); ar=num2cell(a,2); aw=num2cell(a,[1 2]);
                 b=true(3,0); bc=num2cell(b,1); br=num2cell(b,2); bw=num2cell(b,[1 2]);
                 e=zeros(2,0,3); ep=num2cell(e,[2 3]); ec=num2cell(e,[1 3]);
                 s=num2cell(7,1); sf=num2cell(7,4);
                 v=[1 2 3]; vc=num2cell(v,1); vr=num2cell(v,2); vf=num2cell(v,3);
                 t=reshape(1:4,2,1,2); tp=num2cell(t,3); tq=num2cell(t,[2 3]);",
            )
            .unwrap();
        let empty = |dims: &[usize], class: &'static str| (dims.to_vec(), class, Vec::new());
        assert_eq!(
            dense_cells(&engine, "ac"),
            ((1, 3), vec![empty(&[0, 1], "double"); 3])
        );
        assert_eq!(dense_cells(&engine, "ar"), ((0, 1), vec![]));
        assert_eq!(
            dense_cells(&engine, "aw"),
            ((1, 1), vec![empty(&[0, 3], "double")])
        );
        assert_eq!(dense_cells(&engine, "bc"), ((1, 0), vec![]));
        assert_eq!(
            dense_cells(&engine, "br"),
            ((3, 1), vec![empty(&[1, 0], "logical"); 3])
        );
        assert_eq!(
            dense_cells(&engine, "bw"),
            ((1, 1), vec![empty(&[3, 0], "logical")])
        );
        assert_eq!(
            dense_cells(&engine, "ep"),
            ((2, 1), vec![empty(&[1, 0, 3], "double"); 2])
        );
        assert_eq!(dense_cells(&engine, "ec"), ((1, 0), vec![]));
        for name in ["s", "sf"] {
            assert_eq!(
                dense_cells(&engine, name),
                ((1, 1), vec![doubles(&[1, 1], &[7.])]),
                "{name}"
            );
        }
        let scalars = [1., 2., 3.].map(|v| doubles(&[1, 1], &[v]));
        for name in ["vc", "vf"] {
            assert_eq!(
                dense_cells(&engine, name),
                ((1, 3), scalars.to_vec()),
                "{name}"
            );
        }
        assert_eq!(
            dense_cells(&engine, "vr"),
            ((1, 1), vec![doubles(&[1, 3], &[1., 2., 3.])])
        );
        // A singleton middle axis: grouping it changes neither grid nor blocks.
        let pages = [[1., 3.], [2., 4.]].map(|v| doubles(&[1, 1, 2], &v));
        for name in ["tp", "tq"] {
            assert_eq!(
                dense_cells(&engine, name),
                ((2, 1), pages.to_vec()),
                "{name}"
            );
        }
    }

    #[test]
    fn corpus_dense_num2cell_shares_a_whole_block() {
        let mut engine = Engine::default();
        engine
            .execute("b=reshape(1:8,2,2,2); w=num2cell(b,1:3); c=[1;2;3]; x=num2cell(c,1);")
            .unwrap();
        for (source, name) in [("b", "w"), ("c", "x")] {
            let (Some(Array::Numeric(source)), Some(Array::Cell(cells))) =
                (engine.array(source), engine.array(name))
            else {
                panic!("{name}");
            };
            let block = cells.get_linear(0).unwrap();
            let Array::Numeric(block) = block.as_ref() else {
                panic!("{name}");
            };
            assert!(Arc::ptr_eq(&block.storage, &source.storage), "{name}");
            assert_eq!(block.dims(), source.dims(), "{name}");
        }
    }

    // The tiny budgets below are the exact peak: one byte less must refuse
    // before anything is assigned, and the peak itself must succeed.
    fn assert_peak(engine: &mut Engine, call: &str, peak: usize) {
        let mut limits = engine.limits_active();
        limits.memory_bytes = peak - 1;
        engine.limits_set(limits);
        let message = engine.execute(call).unwrap_err();
        assert!(message.contains("num2cell exceeds"), "{call}: {message}");
        assert!(engine.array("c").is_none(), "{call}");
        limits.memory_bytes = peak;
        engine.limits_set(limits);
        engine.execute(call).unwrap();
        assert!(matches!(engine.array("c"), Some(Array::Cell(_))), "{call}");
        engine.execute("clear c").unwrap();
    }

    #[test]
    fn corpus_dense_num2cell_peak_counts_slots_blocks_and_scratch() {
        let mut engine = Engine::default();
        engine
            .execute(
                "a=reshape(1:6,2,3); i=reshape(int8(1:8),2,2,2);
                 l=reshape(logical([1 0 0 1 1 1 0 0]),2,2,2); z=[1+2i 3; 4 5-1i];",
            )
            .unwrap();
        let headers = 2 * size_of::<usize>();
        let digit = 3 * size_of::<usize>();
        let table = size_of::<Vec<Arc<Array>>>() + headers;
        let matrix = size_of::<nalgebra::DMatrix<f64>>();
        let bytes = size_of::<Vec<u8>>();
        // Three 2x1 double columns; one stepped (outer) axis.
        let columns = table + 3 * (SLOT + 2 * headers + matrix) + 6 * size_of::<f64>() + digit;
        assert_peak(&mut engine, "c=num2cell(a,1);", columns);
        // Two 1x3 double rows gather one element at a time: two stepped axes.
        let rows = table + 2 * (SLOT + 2 * headers + matrix) + 6 * size_of::<f64>() + 2 * digit;
        assert_peak(&mut engine, "c=num2cell(a,2);", rows);
        // Two 2x1x2 int8 blocks: one payload byte per element, and each N-D
        // block retains a shared 3-entry dimension vector; the walk holds two
        // stepped axes and the block dimensions.
        let shape = headers + size_of::<Vec<usize>>() + 3 * size_of::<usize>();
        let scratch = 2 * digit + 3 * size_of::<usize>();
        let typed = table + 2 * (SLOT + 2 * headers + bytes + shape) + 8 + scratch;
        assert_peak(&mut engine, "c=num2cell(i,[1 3]);", typed);
        // Logical blocks keep one byte per element as well.
        assert_peak(&mut engine, "c=num2cell(l,[1 3]);", typed);
        // Complex columns keep sixteen bytes per element.
        let complex = table + 2 * (SLOT + 2 * headers + matrix) + 4 * 16 + digit;
        assert_peak(&mut engine, "c=num2cell(z,1);", complex);
    }

    #[test]
    fn corpus_dense_num2cell_stops_inside_one_block() {
        let engine = Engine::default();
        // A 2x3x2 source grouped along [1 3]: three blocks of two 2-element runs.
        let source: Vec<f64> = (1..=12).map(f64::from).collect();
        let layout = (size_of::<f64>(), size_of::<nalgebra::DMatrix<f64>>());
        let run = |trip: usize| {
            let mut plan = dense_plan(&engine, &[2, 3, 2], 0b101, layout).unwrap();
            assert_eq!(
                (plan.walk.count, plan.walk.block_len, plan.walk.run),
                (3, 4, 2)
            );
            let calls = Cell::new(0);
            // An injected threshold of three elements puts a checkpoint in the
            // middle of each block's second run.
            let stop = || {
                calls.set(calls.get() + 1);
                if calls.get() == trip {
                    engine.cancel.store(true, Ordering::Relaxed);
                }
                engine.check_cancel()
            };
            let mut blocks = Vec::new();
            let result = dense_blocks(&source, &mut plan.walk, 3, &stop, |block| {
                blocks.push(block);
                Ok(())
            });
            engine.cancel.store(false, Ordering::Relaxed);
            (result, blocks, calls.get())
        };
        let (result, blocks, calls) = run(0);
        result.unwrap();
        assert_eq!(
            blocks,
            [[1., 2., 7., 8.], [3., 4., 9., 10.], [5., 6., 11., 12.]]
        );
        // One checkpoint at each block start and one after its third element.
        assert_eq!(calls, 6);
        // Stop at the second checkpoint: three of four elements were gathered
        // and the unfinished first block is never emitted.
        let (result, blocks, calls) = run(2);
        assert_eq!(result.unwrap_err(), "Calculation cancelled");
        assert!(blocks.is_empty());
        assert_eq!(calls, 2);
        // Stop inside the second block keeps exactly the completed first one.
        let (result, blocks, calls) = run(4);
        assert_eq!(result.unwrap_err(), "Calculation cancelled");
        assert_eq!(blocks, [[1., 2., 7., 8.]]);
        assert_eq!(calls, 4);
    }
}
