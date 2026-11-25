//! Process-local graphics identities and immutable, nonnumeric handle arrays.
use crate::{
    Result,
    script_containers::{CellTable, MAX_CELL_SLOTS},
};
use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

/// A model snapshot clone retains object identity. A constructor/import creates
/// a new identity. This token is deliberately not serialized.
#[derive(Clone)]
pub struct ObjectIdentity(Arc<u64>);
impl Default for ObjectIdentity {
    fn default() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(Arc::new(NEXT.fetch_add(1, Ordering::Relaxed)))
    }
}
impl ObjectIdentity {
    pub(crate) fn recovery_key(&self) -> usize {
        Arc::as_ptr(&self.0) as usize
    }
    pub(crate) fn creation_order(&self) -> u64 {
        *self.0
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
    /// An unfilled element of a graphics handle array.
    Placeholder,
    Root,
    Figure,
    Axes,
    Series,
    TextBox,
    Annotation,
    Legend,
    Baseline,
    /// A text object, axes title/label or colorbar label.
    Text,
    Colorbar,
    /// An image object (`image`, `imagesc`, `imshow`).
    Image,
    /// A 3-D volume object (`volshow`).
    Volume,
    /// A 2-D slice viewer (`sliceViewer`).
    SliceViewer,
    /// An orthogonal 3-pane slice viewer (`orthosliceViewer`).
    OrthosliceViewer,
    TiledLayout,
    /// A numeric axis ruler (`ax.XAxis`, `ax.YAxis`, `ax.ZAxis`).
    NumericRuler,
    /// A constant horizontal or vertical line (`xline`, `yline`).
    ConstantLine,
    /// A 2-D rectangle/ellipse (`rectangle`).
    Rectangle,
    /// A UI component or UI figure (`uifigure`, `uibutton`, ...).
    UiComponent,
}

impl GraphicsKind {
    pub(crate) fn has_mutable_parent(self) -> bool {
        matches!(
            self,
            Self::Volume | Self::SliceViewer | Self::OrthosliceViewer | Self::UiComponent
        )
    }
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
        if self.kind == GraphicsKind::Root {
            return false;
        }
        if self.kind == GraphicsKind::Volume {
            return figure.all_axes().take(16).any(|(_, axes)| {
                axes.extras
                    .volumes
                    .iter()
                    .take(crate::plot::MAX_VOLUMES)
                    .any(|volume| self.object.same_object(&volume.identity))
            });
        }
        if !self.kind.has_mutable_parent() && !self.figure.same_object(&figure.identity) {
            return false;
        }
        match self.kind {
            GraphicsKind::Figure => self.object.same_object(&figure.identity),
            GraphicsKind::TiledLayout => figure
                .layout
                .as_ref()
                .is_some_and(|l| l.tiled.is_some() && self.object.same_object(&l.identity)),
            GraphicsKind::Text if self.axes.is_none() => figure.layout.as_ref().is_some_and(|l| {
                self.object.same_object(&l.title_identity)
                    || self.object.same_object(&l.xlabel_identity)
                    || self.object.same_object(&l.ylabel_identity)
            }),
            GraphicsKind::TextBox => figure
                .textboxes
                .iter()
                .take(64)
                .any(|textbox| self.object.same_object(&textbox.identity)),
            GraphicsKind::SliceViewer => figure.slice_viewers.iter().take(16).any(|sv| {
                self.object.same_object(&sv.identity)
                    && figure
                        .all_axes()
                        .take(16)
                        .any(|(_, a)| a.identity.same_object(&sv.axes_identity))
            }),
            GraphicsKind::OrthosliceViewer => figure.orthoslice_viewers.iter().take(16).any(|ov| {
                self.object.same_object(&ov.identity)
                    && ov.axes_identities.iter().all(|id| {
                        figure
                            .all_axes()
                            .take(16)
                            .any(|(_, a)| a.identity.same_object(id))
                    })
            }),
            GraphicsKind::Annotation => figure
                .annotations
                .iter()
                .take(64)
                .any(|ann| self.object.same_object(&ann.identity)),
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
                    GraphicsKind::Text
                    | GraphicsKind::Colorbar
                    | GraphicsKind::Image
                    | GraphicsKind::NumericRuler
                    | GraphicsKind::ConstantLine
                    | GraphicsKind::Rectangle => axes.extras.owns(&self.object),
                    _ => false,
                }
            }),
        }
    }

    pub fn kind(&self) -> GraphicsKind {
        self.kind
    }
    pub fn same_object(&self, other: &Self) -> bool {
        if self.kind == other.kind
            && matches!(self.kind, GraphicsKind::Root | GraphicsKind::Placeholder)
        {
            return true;
        }
        if self.kind == other.kind && self.kind.has_mutable_parent() {
            return self.object.same_object(&other.object);
        }
        self.kind == other.kind
            && self.figure.same_object(&other.figure)
            && self.object.same_object(&other.object)
            && match (&self.axes, &other.axes) {
                (None, None) => true,
                (Some(a), Some(b)) => a.same_object(b),
                _ => false,
            }
    }
    pub(crate) fn placeholder() -> Self {
        Self {
            kind: GraphicsKind::Placeholder,
            ..Self::root()
        }
    }
    fn numeric_value(&self) -> f64 {
        match self.kind {
            GraphicsKind::Placeholder => f64::NAN,
            GraphicsKind::Root => 0.,
            // Negative process-local IDs stay distinct from numeric figure numbers.
            _ => -(self.object.creation_order() as f64),
        }
    }
    pub(crate) fn root() -> Self {
        Self {
            figure: ObjectIdentity::default(),
            axes: None,
            object: ObjectIdentity::default(),
            kind: GraphicsKind::Root,
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
        Self::figure_object(figure, id, GraphicsKind::TextBox)
    }
    pub(crate) fn figure_object(
        figure: &ObjectIdentity,
        id: &ObjectIdentity,
        kind: GraphicsKind,
    ) -> Self {
        Self {
            figure: figure.clone(),
            axes: None,
            object: id.clone(),
            kind,
        }
    }
    pub(crate) fn annotation(figure: &ObjectIdentity, id: &ObjectIdentity) -> Self {
        Self {
            figure: figure.clone(),
            axes: None,
            object: id.clone(),
            kind: GraphicsKind::Annotation,
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
    pub(crate) fn tiled_layout(figure: &ObjectIdentity, id: &ObjectIdentity) -> Self {
        Self {
            figure: figure.clone(),
            axes: None,
            object: id.clone(),
            kind: GraphicsKind::TiledLayout,
        }
    }
    pub(crate) fn layout_text(figure: &ObjectIdentity, id: &ObjectIdentity) -> Self {
        Self {
            figure: figure.clone(),
            axes: None,
            object: id.clone(),
            kind: GraphicsKind::Text,
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
    /// Volumes can move between axes and figures. Their handles name the
    /// volume itself, and its owner is resolved in the current graph.
    pub(crate) fn volume(id: &ObjectIdentity) -> Self {
        Self::figure_object(id, id, GraphicsKind::Volume)
    }
    /// A UI component or UI figure stored in `Engine::ui`.
    pub(crate) fn ui_component(id: &ObjectIdentity) -> Self {
        Self::figure_object(id, id, GraphicsKind::UiComponent)
    }
    /// Axes-owned text, title/label or colorbar (see `plot::AxesExtras`).
    pub(crate) fn axes_object(
        figure: &ObjectIdentity,
        axes: &ObjectIdentity,
        id: &ObjectIdentity,
        kind: GraphicsKind,
    ) -> Self {
        Self {
            figure: figure.clone(),
            axes: Some(axes.clone()),
            object: id.clone(),
            kind,
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
    empty_class: Option<&'static str>,
}
impl GraphicsArray {
    pub(crate) fn to_double(&self, cancel: &AtomicBool) -> Result<crate::Matrix> {
        let mut values = Vec::with_capacity(self.len());
        for i in 0..self.len() {
            if i.is_multiple_of(1024) {
                crate::dsp::check_cancel(cancel)?;
            }
            values.push(self.get_linear(i)?.numeric_value());
        }
        crate::Matrix::from_column_vec(self.rows(), self.cols(), values)
    }
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
            empty_class: self.empty_class,
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
            empty_class: None,
        })
    }
    pub(crate) fn empty_typed(
        rows: usize,
        cols: usize,
        class_name: &'static str,
        cancel: &AtomicBool,
    ) -> Result<Self> {
        Ok(Self {
            table: CellTable::from_column_major(rows, cols, Vec::new(), cancel)?,
            empty_class: Some(class_name),
        })
    }
    pub fn empty_class(&self) -> Option<&'static str> {
        self.empty_class
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
            empty_class: None,
        })
    }
    pub fn preview(&self, _limit: usize) -> String {
        if let Some(cls) = self.empty_class
            && self.is_empty()
        {
            return format!("{}x{} empty {cls} array", self.rows(), self.cols());
        }
        format!("{}x{} graphics handle array", self.rows(), self.cols())
    }
}
impl fmt::Display for GraphicsArray {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(cls) = self.empty_class
            && self.is_empty()
        {
            return write!(f, "{}x{} empty {cls} array", self.rows(), self.cols());
        }
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
