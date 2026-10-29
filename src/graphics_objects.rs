//! Process-local graphics identities and immutable, nonnumeric handle arrays.
use crate::{
    Result,
    script_containers::{CellTable, MAX_CELL_SLOTS},
};
use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

/// A model snapshot clone retains object identity. A constructor/import creates
/// a new identity. This token is deliberately not serialized or exposed as f64.
#[derive(Clone, Default)]
pub struct ObjectIdentity(Arc<()>);
impl ObjectIdentity {
    pub(crate) fn recovery_key(&self) -> usize {
        Arc::as_ptr(&self.0) as usize
    }
    pub fn same_object(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl fmt::Debug for ObjectIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ObjectIdentity")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphicsKind {
    Figure,
    Axes,
    Series,
    TextBox,
    Legend,
    Baseline,
}

/// Opaque reference with parent provenance. No model/sample allocation is held.
#[derive(Clone, Debug)]
pub struct GraphicsHandle {
    pub(crate) figure: ObjectIdentity,
    pub(crate) axes: Option<ObjectIdentity>,
    pub(crate) object: ObjectIdentity,
    pub(crate) kind: GraphicsKind,
}
impl GraphicsHandle {
    /// Inspect liveness against one validated figure snapshot without scanning
    /// sample payloads or retaining the figure. The caller supplies its current
    /// snapshot; this is not a query of another process or a future worker reply.
    pub fn is_live_in(&self, figure: &crate::plot::Figure) -> bool {
        if !self.figure.same_object(&figure.identity) {
            return false;
        }
        match self.kind {
            GraphicsKind::Figure => self.object.same_object(&figure.identity),
            GraphicsKind::TextBox => figure
                .textboxes
                .iter()
                .take(64)
                .any(|textbox| self.object.same_object(&textbox.identity)),
            _ => figure.all_axes().take(16).any(|(_, axes)| {
                if !self
                    .axes
                    .as_ref()
                    .is_some_and(|id| id.same_object(&axes.identity))
                {
                    return false;
                }
                match self.kind {
                    GraphicsKind::Axes => self.object.same_object(&axes.identity),
                    GraphicsKind::Series => axes
                        .series
                        .iter()
                        .take(64)
                        .any(|series| self.object.same_object(&series.identity)),
                    GraphicsKind::Baseline => axes.observe_baseline(&self.object).is_ok(),
                    GraphicsKind::Legend => axes
                        .legend_state
                        .as_ref()
                        .is_some_and(|legend| self.object.same_object(&legend.identity)),
                    _ => false,
                }
            }),
        }
    }

    pub fn kind(&self) -> GraphicsKind {
        self.kind
    }
    pub fn same_object(&self, other: &Self) -> bool {
        self.kind == other.kind
            && self.figure.same_object(&other.figure)
            && self.object.same_object(&other.object)
            && match (&self.axes, &other.axes) {
                (None, None) => true,
                (Some(a), Some(b)) => a.same_object(b),
                _ => false,
            }
    }
    pub(crate) fn baseline(
        figure: &ObjectIdentity,
        axes: &ObjectIdentity,
        id: &ObjectIdentity,
    ) -> Self {
        Self {
            figure: figure.clone(),
            axes: Some(axes.clone()),
            object: id.clone(),
            kind: GraphicsKind::Baseline,
        }
    }
    pub(crate) fn legend(
        figure: &ObjectIdentity,
        axes: &ObjectIdentity,
        id: &ObjectIdentity,
    ) -> Self {
        Self {
            figure: figure.clone(),
            axes: Some(axes.clone()),
            object: id.clone(),
            kind: GraphicsKind::Legend,
        }
    }
    pub(crate) fn textbox(figure: &ObjectIdentity, id: &ObjectIdentity) -> Self {
        Self {
            figure: figure.clone(),
            axes: None,
            object: id.clone(),
            kind: GraphicsKind::TextBox,
        }
    }
    pub(crate) fn figure(id: &ObjectIdentity) -> Self {
        Self {
            figure: id.clone(),
            axes: None,
            object: id.clone(),
            kind: GraphicsKind::Figure,
        }
    }
    pub(crate) fn axes(figure: &ObjectIdentity, id: &ObjectIdentity) -> Self {
        Self {
            figure: figure.clone(),
            axes: Some(id.clone()),
            object: id.clone(),
            kind: GraphicsKind::Axes,
        }
    }
    pub(crate) fn series(
        figure: &ObjectIdentity,
        axes: &ObjectIdentity,
        id: &ObjectIdentity,
    ) -> Self {
        Self {
            figure: figure.clone(),
            axes: Some(axes.clone()),
            object: id.clone(),
            kind: GraphicsKind::Series,
        }
    }
}

#[derive(Clone, Debug)]
pub struct GraphicsArray {
    table: CellTable<GraphicsHandle>,
}
impl GraphicsArray {
    pub(crate) fn recovery_key(&self) -> usize {
        self.table.weak_storage().as_ptr() as usize
    }
    pub(crate) fn reshape_recovery(
        &self,
        rows: usize,
        cols: usize,
        cancel: &AtomicBool,
    ) -> Result<Self> {
        Ok(Self {
            table: self.table.reshape(rows, cols, cancel)?,
        })
    }
    pub fn from_shared(
        rows: usize,
        cols: usize,
        values: Vec<Arc<GraphicsHandle>>,
        cancel: &AtomicBool,
    ) -> Result<Self> {
        Ok(Self {
            table: CellTable::from_column_major(rows, cols, values, cancel)?,
        })
    }
    pub(crate) fn scalar(handle: GraphicsHandle, cancel: &AtomicBool) -> Result<Self> {
        Self::from_shared(1, 1, vec![Arc::new(handle)], cancel)
    }
    pub fn rows(&self) -> usize {
        self.table.rows()
    }
    pub fn cols(&self) -> usize {
        self.table.cols()
    }
    pub fn len(&self) -> usize {
        self.table.len()
    }
    pub fn is_empty(&self) -> bool {
        self.table.is_empty()
    }
    pub fn bytes(&self) -> usize {
        // Repeated references charged repeatedly; conservative logical payload,
        // not allocator/RSS. Includes three possible Arc identity allocations.
        self.len()
            * (std::mem::size_of::<GraphicsHandle>()
                + std::mem::size_of::<Arc<GraphicsHandle>>()
                + 8 * std::mem::size_of::<usize>())
    }
    pub fn same_storage(&self, other: &Self) -> bool {
        self.table.same_storage(&other.table)
    }
    pub fn get_linear(&self, index: usize) -> Result<&Arc<GraphicsHandle>> {
        if index >= self.len() {
            return Err("Graphics handle index is out of range".into());
        }
        self.table.get(index % self.rows(), index / self.rows())
    }
    pub fn scalar_handle(&self) -> Result<&GraphicsHandle> {
        if self.len() != 1 {
            return Err("This graphics operation requires one scalar handle".into());
        }
        Ok(self.get_linear(0)?.as_ref())
    }
    pub fn selected(
        &self,
        rows: usize,
        cols: usize,
        indices: &[usize],
        cancel: &AtomicBool,
    ) -> Result<Self> {
        crate::dsp::check_cancel(cancel)?;
        if rows > MAX_CELL_SLOTS
            || cols > MAX_CELL_SLOTS
            || rows.checked_mul(cols) != Some(indices.len())
            || indices.len() > MAX_CELL_SLOTS
        {
            return Err("Graphics handle selection exceeds bounded shape".into());
        }
        let mut values = Vec::new();
        values
            .try_reserve_exact(indices.len())
            .map_err(|_| "Cannot allocate graphics handle selection")?;
        for (i, &index) in indices.iter().enumerate() {
            if i.is_multiple_of(1024) && cancel.load(Ordering::Relaxed) {
                return Err("Calculation cancelled".into());
            }
            values.push(self.get_linear(index)?.clone());
        }
        Self::from_shared(rows, cols, values, cancel)
    }
    pub fn replaced(
        &self,
        index: usize,
        value: Arc<GraphicsHandle>,
        cancel: &AtomicBool,
    ) -> Result<Self> {
        self.get_linear(index)?;
        Ok(Self {
            table: self
                .table
                .replaced(index % self.rows(), index / self.rows(), value, cancel)?,
        })
    }
    pub fn preview(&self, _limit: usize) -> String {
        format!("{}x{} graphics handle array", self.rows(), self.cols())
    }
}
impl fmt::Display for GraphicsArray {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}x{} graphics handle array", self.rows(), self.cols())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn immutable_handle_arrays_bound_shape_capacity_and_cancel_without_losing_identity() {
        let cancel = AtomicBool::new(false);
        let id = ObjectIdentity::default();
        let a = GraphicsArray::scalar(GraphicsHandle::figure(&id), &cancel).unwrap();
        let shared = a.clone();
        assert!(a.same_storage(&shared));
        let empty = GraphicsArray::from_shared(0, 17, Vec::new(), &cancel).unwrap();
        assert_eq!((empty.rows(), empty.cols(), empty.len()), (0, 17, 0));
        assert!(empty.get_linear(0).is_err());
        assert!(GraphicsArray::from_shared(0, MAX_CELL_SLOTS + 1, Vec::new(), &cancel).is_err());
        assert!(
            GraphicsArray::from_shared(0, 0, Vec::with_capacity(MAX_CELL_SLOTS + 1), &cancel)
                .is_err()
        );
        cancel.store(true, Ordering::Relaxed);
        assert!(
            a.selected(1, 1, &[0], &cancel)
                .unwrap_err()
                .contains("cancel")
        );
        assert!(
            a.replaced(0, a.get_linear(0).unwrap().clone(), &cancel)
                .unwrap_err()
                .contains("cancel")
        );
        assert!(a.same_storage(&shared));
        cancel.store(false, Ordering::Relaxed);
        let selected = a.selected(1, 2, &[0, 0], &cancel).unwrap();
        assert!(!a.same_storage(&selected));
        for i in 0..2 {
            assert!(
                selected
                    .get_linear(i)
                    .unwrap()
                    .same_object(a.scalar_handle().unwrap())
            );
        }
    }
}
