//! Figure-level slice viewer settings and binary source volumes. Pane images
//! and crosshairs are derived by the same code used by live viewers.
use super::{VolumeDataClass, take_native_f64, write_native_f64};
use crate::plot::coordinates::{GeometryCost, GeometryLedger};
use crate::{
    ObjectIdentity, Result,
    plot::{Axes, Figure, OrthosliceViewerObject, SliceDirection, SliceViewerObject},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ViewerMetadata {
    selection: Selection,
    dims: [usize; 3],
    data_class: VolumeDataClass,
    colormap_rows: usize,
    display_range: Option<[f64; 2]>,
    scale_factors: [f64; 3],
    visible: bool,
    tag: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
enum Selection {
    Slice {
        pane: Pane,
        number: usize,
        #[serde(with = "Direction")]
        direction: SliceDirection,
    },
    Ortho {
        /// XY, YZ and XZ, in that order, independent of layout order.
        panes: [Pane; 3],
        numbers: [usize; 3],
        crosshair_enabled: bool,
        crosshair_slots: [[Option<usize>; 2]; 3],
    },
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "SliceDirection")]
enum Direction {
    X,
    Y,
    Z,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pane {
    axes: usize,
    shared_colormap: bool,
    y_reversed: bool,
    equal_aspect: bool,
    axis_off: bool,
}

impl Selection {
    fn panes(&self) -> &[Pane] {
        match self {
            Self::Slice { pane, .. } => std::slice::from_ref(pane),
            Self::Ortho { panes, .. } => panes,
        }
    }
}

impl Pane {
    fn capture(index: usize, axes: &Axes, colormap: &Arc<Vec<[f64; 3]>>) -> Self {
        let extras = &axes.extras;
        Self {
            axes: index,
            shared_colormap: extras
                .colormap
                .as_ref()
                .is_some_and(|m| Arc::ptr_eq(m, colormap)),
            y_reversed: extras.y_reversed,
            equal_aspect: extras.equal_aspect,
            axis_off: extras.axis_off,
        }
    }

    fn rebuild(&self, axes: &mut Axes, sync: impl FnOnce(&mut Axes)) -> Result<()> {
        let colormap = axes.extras.colormap.take();
        if self.shared_colormap && colormap.is_some() {
            return Err("Conflicting native slice viewer pane colormap".into());
        }
        let clim = axes.extras.clim;
        sync(axes);
        if !self.shared_colormap {
            axes.extras.colormap = colormap;
        }
        axes.extras.clim = clim;
        axes.extras.y_reversed = self.y_reversed;
        axes.extras.equal_aspect = self.equal_aspect;
        axes.extras.axis_off = self.axis_off;
        Ok(())
    }
}

pub(super) fn capture(figure: &Figure, axes: &[&Axes]) -> Result<Vec<ViewerMetadata>> {
    let pane_index = |id: &ObjectIdentity| {
        axes.iter()
            .position(|a| a.identity.same_object(id))
            .ok_or_else(|| "Slice viewer refers to a missing axes".to_owned())
    };
    if figure.slice_viewers.len() + figure.orthoslice_viewers.len() > axes.len() {
        return Err("Slice viewer count exceeds figure axes count".into());
    }
    let mut metadata = Vec::new();
    for viewer in &figure.slice_viewers {
        let index = pane_index(&viewer.axes_identity)?;
        metadata.push(ViewerMetadata {
            selection: Selection::Slice {
                pane: Pane::capture(index, axes[index], &viewer.colormap),
                number: viewer.slice_number,
                direction: viewer.slice_direction,
            },
            dims: viewer.dims,
            data_class: VolumeDataClass::from_image_class(viewer.data_class),
            colormap_rows: viewer.colormap.len(),
            display_range: viewer.display_range,
            scale_factors: viewer.scale_factors,
            visible: viewer.visible,
            tag: viewer.tag.clone(),
        });
    }
    for viewer in &figure.orthoslice_viewers {
        let [xy, yz, xz] = &viewer.axes_identities;
        let indices = [pane_index(xy)?, pane_index(yz)?, pane_index(xz)?];
        let mut crosshair_slots = [[None; 2]; 3];
        for (pane, &index) in indices.iter().enumerate() {
            for (slot, id) in crosshair_slots[pane]
                .iter_mut()
                .zip(viewer.pane_graphics(pane).1)
            {
                *slot = axes[index]
                    .series
                    .iter()
                    .position(|s| s.identity.same_object(id));
            }
        }
        metadata.push(ViewerMetadata {
            selection: Selection::Ortho {
                panes: indices.map(|i| Pane::capture(i, axes[i], &viewer.colormap)),
                numbers: viewer.slice_numbers,
                crosshair_enabled: viewer.crosshair_enabled,
                crosshair_slots,
            },
            dims: viewer.dims,
            data_class: VolumeDataClass::from_image_class(viewer.data_class),
            colormap_rows: viewer.colormap.len(),
            display_range: viewer.display_range,
            scale_factors: viewer.scale_factors,
            visible: viewer.visible,
            tag: viewer.tag.clone(),
        });
    }
    if !figure.orthoslice_viewers.is_empty() {
        let mut ledger = GeometryLedger::default();
        let crosshair = GeometryCost::ordinary_allocation(2, false)?;
        for (index, axes) in axes.iter().enumerate() {
            if let Some(slots) = pane_crosshair_slots(&metadata, index) {
                for (slot, series) in axes.series.iter().enumerate() {
                    if !slots.contains(&Some(slot)) {
                        ledger.add(series.geometry_cost()?)?;
                    }
                }
                ledger.add(crosshair)?;
                ledger.add(crosshair)?;
            } else {
                ledger.merge(axes.geometry_ledger()?)?;
            }
        }
    }
    Ok(metadata)
}

pub(super) fn shares_colormap(metadata: &[ViewerMetadata], index: usize) -> bool {
    metadata
        .iter()
        .flat_map(|m| m.selection.panes())
        .any(|p| p.axes == index && p.shared_colormap)
}

fn pane_crosshair_slots(metadata: &[ViewerMetadata], index: usize) -> Option<[Option<usize>; 2]> {
    metadata.iter().find_map(|viewer| {
        let Selection::Ortho {
            panes,
            crosshair_slots,
            ..
        } = &viewer.selection
        else {
            return None;
        };
        panes
            .iter()
            .position(|p| p.axes == index)
            .map(|pane| crosshair_slots[pane])
    })
}

pub(super) fn prepare_pane(
    metadata: &[ViewerMetadata],
    index: usize,
    axes: &mut Axes,
) -> Result<()> {
    if shares_colormap(metadata, index) {
        axes.extras.colormap = None;
    }
    if let Some(mut slots) = pane_crosshair_slots(metadata, index) {
        slots.sort_unstable();
        for slot in slots.into_iter().rev().flatten() {
            axes.remove_series(slot)?;
        }
    }
    Ok(())
}

pub(super) fn payload_size(metadata: &[ViewerMetadata], axes_count: usize) -> Result<usize> {
    if metadata.len() > axes_count {
        return Err("Slice viewer count exceeds figure axes count".into());
    }
    let mut occupied = [false; 16];
    let mut size = 0usize;
    for viewer in metadata {
        for pane in viewer.selection.panes() {
            if pane.axes >= axes_count
                || pane.axes >= occupied.len()
                || std::mem::replace(&mut occupied[pane.axes], true)
            {
                return Err("Invalid or repeated native slice viewer pane index".into());
            }
        }
        let voxels = viewer.voxel_count()?;
        if !(1..=crate::plot::MAX_COLORMAP_ROWS).contains(&viewer.colormap_rows) {
            return Err("Invalid native slice viewer colormap size".into());
        }
        size = size
            .checked_add((voxels + viewer.colormap_rows * 3) * 8)
            .ok_or("Native slice viewer payload extent overflow")?;
    }
    Ok(size)
}

pub(super) fn write_payload(out: &mut Vec<u8>, figure: &Figure) {
    for viewer in &figure.slice_viewers {
        write_native_f64(out, viewer.data.iter());
        write_native_f64(out, viewer.colormap.iter().flatten());
    }
    for viewer in &figure.orthoslice_viewers {
        write_native_f64(out, viewer.data.iter());
        write_native_f64(out, viewer.colormap.iter().flatten());
    }
}

impl ViewerMetadata {
    fn voxel_count(&self) -> Result<usize> {
        let count = self.dims.into_iter().try_fold(1usize, usize::checked_mul);
        count
            .filter(|n| (1..=crate::plot::MAX_VOLUME_VOXELS).contains(n))
            .ok_or_else(|| "Invalid native slice viewer dimensions or voxel count".into())
    }

    fn restore(
        self,
        payload: &mut &[u8],
        figure: &mut Figure,
        axes_ids: &[ObjectIdentity],
    ) -> Result<()> {
        let data = take_native_f64(payload, self.voxel_count()?).collect();
        let mut values = take_native_f64(payload, self.colormap_rows * 3);
        let colormap = Arc::new(
            (0..self.colormap_rows)
                .map(|_| std::array::from_fn(|_| values.next().unwrap()))
                .collect(),
        );
        let data_class = self.data_class.to_image_class();
        match self.selection {
            Selection::Slice {
                pane,
                number,
                direction,
            } => {
                let axes = find_pane(figure, &axes_ids[pane.axes])?;
                let mut viewer =
                    SliceViewerObject::new(self.dims, data, data_class, axes.identity.clone());
                viewer.slice_number = number;
                viewer.slice_direction = direction;
                viewer.colormap = colormap;
                viewer.display_range = self.display_range;
                viewer.scale_factors = self.scale_factors;
                viewer.visible = self.visible;
                viewer.tag = self.tag;
                viewer.validate()?;
                pane.rebuild(axes, |axes| viewer.sync_axes(axes))?;
                figure.slice_viewers.push(viewer);
            }
            Selection::Ortho {
                panes,
                numbers,
                crosshair_enabled,
                crosshair_slots,
            } => {
                let mut viewer = OrthosliceViewerObject::new(
                    self.dims,
                    data,
                    data_class,
                    panes.each_ref().map(|p| axes_ids[p.axes].clone()),
                );
                viewer.slice_numbers = numbers;
                viewer.crosshair_enabled = crosshair_enabled;
                viewer.colormap = colormap;
                viewer.display_range = self.display_range;
                viewer.scale_factors = self.scale_factors;
                viewer.visible = self.visible;
                viewer.tag = self.tag;
                viewer.validate()?;
                for (i, pane) in panes.iter().enumerate() {
                    let axes = find_pane(figure, &viewer.axes_identities[i])?;
                    let slots = crosshair_slots[i];
                    let stored_count = slots.iter().flatten().count();
                    if (slots[0].is_some() && slots[0] == slots[1])
                        || slots
                            .iter()
                            .flatten()
                            .any(|&slot| slot >= axes.series.len() + stored_count)
                    {
                        return Err(
                            "Invalid native slice viewer crosshair slots or series count".into(),
                        );
                    }
                    pane.rebuild(axes, |axes| viewer.sync_pane(i, axes))?;
                    let horizontal = axes.series.pop().unwrap();
                    let vertical = axes.series.pop().unwrap();
                    let mut lines = [(slots[0], vertical), (slots[1], horizontal)];
                    lines.sort_by_key(|(slot, _)| slot.unwrap_or(usize::MAX));
                    for (slot, line) in lines {
                        axes.series.insert(slot.unwrap_or(axes.series.len()), line);
                    }
                }
                figure.orthoslice_viewers.push(viewer);
            }
        }
        Ok(())
    }
}

fn find_pane<'a>(figure: &'a mut Figure, id: &ObjectIdentity) -> Result<&'a mut Axes> {
    figure
        .all_axes_mut()
        .find(|a| a.identity.same_object(id))
        .ok_or_else(|| "Native slice viewer refers to a missing axes".into())
}

pub(super) fn restore(
    metadata: Vec<ViewerMetadata>,
    mut payload: &[u8],
    figure: &mut Figure,
    axes_ids: &[ObjectIdentity],
) -> Result<()> {
    if metadata
        .iter()
        .any(|viewer| matches!(viewer.selection, Selection::Ortho { .. }))
    {
        let mut ledger = figure.geometry_ledger()?;
        let crosshair = GeometryCost::ordinary_allocation(2, false)?;
        for _ in metadata
            .iter()
            .filter(|viewer| matches!(viewer.selection, Selection::Ortho { .. }))
        {
            for _ in 0..6 {
                ledger.add(crosshair)?;
            }
        }
    }
    for viewer in metadata {
        viewer.restore(&mut payload, figure, axes_ids)?;
    }
    debug_assert!(payload.is_empty());
    Ok(())
}
