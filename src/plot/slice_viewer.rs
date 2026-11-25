//! 2-D slice viewers (`sliceViewer` and `orthosliceViewer`) backed by
//! `ImageObject` slices extracted on demand from shared 3-D volume storage.
use super::{
    Axes, ImageDataClass, ImageMapping, ImageObject, MAX_COLORMAP_ROWS, PlotKind, Series,
    volume::{MAX_VOLUME_VOXELS, default_colormap, finite_min_max},
};
use crate::{GraphicsHandle, GraphicsKind, ObjectIdentity, Result};
use std::sync::Arc;

const CROSSHAIR_COLOR: [u8; 3] = [0, 210, 255];

/// Slicing axis for `sliceViewer`: `[1 0 0]` (`'X'`), `[0 1 0]` (`'Y'`), or `[0 0 1]` (`'Z'`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SliceDirection {
    X,
    Y,
    #[default]
    Z,
}

impl SliceDirection {
    pub fn as_vec(self) -> [f64; 3] {
        match self {
            Self::X => [1.0, 0.0, 0.0],
            Self::Y => [0.0, 1.0, 0.0],
            Self::Z => [0.0, 0.0, 1.0],
        }
    }

    pub fn axis_len(self, dims: [usize; 3]) -> usize {
        let [ny, nx, nz] = dims;
        match self {
            Self::X => nx,
            Self::Y => ny,
            Self::Z => nz,
        }
    }

    pub fn default_slice(self, dims: [usize; 3]) -> usize {
        self.axis_len(dims).div_ceil(2)
    }
}

/// Extract a single 2-D slice from column-major volume data `y + ny * (x + nx * z)`
/// without copying the full volume.
///
/// Orientation rules (matching MATLAB's `sliceViewer` / `orthosliceViewer`):
/// - `SliceDirection::Z` (`[0 0 1]`, XY plane at `z`): `V(:, :, z)`, `ny` rows (Y) by `nx` cols (X).
/// - `SliceDirection::X` (`[1 0 0]`, YZ plane at `x`): `squeeze(V(:, x, :))`, `ny` rows (Y) by `nz` cols (Z).
/// - `SliceDirection::Y` (`[0 1 0]`, XZ plane at `y`): `squeeze(V(y, :, :))'`, `nz` rows (Z) by `nx` cols (X),
///   transposed so X is horizontal and Z is vertical (consistent with the `orthosliceViewer` XZ pane).
pub fn extract_slice_image(
    dims: [usize; 3],
    data: &[f64],
    data_class: ImageDataClass,
    direction: SliceDirection,
    slice_1based: usize,
    scale_factors: [f64; 3],
    visible: bool,
) -> ImageObject {
    let [ny, nx, nz] = dims;
    let [sx, sy, sz] = scale_factors;
    let slice = slice_1based.saturating_sub(1);

    let (rows, cols, out, x_span, y_span) = match direction {
        SliceDirection::Z => {
            let z = slice.min(nz.saturating_sub(1));
            let base = ny * nx * z;
            let len = ny * nx;
            let out = data[base..base + len].to_vec();
            (ny, nx, out, [sx, nx as f64 * sx], [sy, ny as f64 * sy])
        }
        SliceDirection::X => {
            let x = slice.min(nx.saturating_sub(1));
            let mut out = Vec::with_capacity(ny * nz);
            for z in 0..nz {
                let start = ny * (x + nx * z);
                out.extend_from_slice(&data[start..start + ny]);
            }
            (ny, nz, out, [sz, nz as f64 * sz], [sy, ny as f64 * sy])
        }
        SliceDirection::Y => {
            let y = slice.min(ny.saturating_sub(1));
            let mut out = Vec::with_capacity(nz * nx);
            for x in 0..nx {
                for z in 0..nz {
                    out.push(data[y + ny * (x + nx * z)]);
                }
            }
            (nz, nx, out, [sx, nx as f64 * sx], [sz, nz as f64 * sz])
        }
    };

    let mut img = ImageObject::new(rows, cols, Arc::from(out), ImageMapping::Scaled);
    img.data_class = data_class;
    img.x = x_span;
    img.y = y_span;
    img.visible = visible;
    img
}

fn validate_volume_core(
    dims: [usize; 3],
    data: &[f64],
    colormap: &[[f64; 3]],
    display_range: Option<[f64; 2]>,
    scale_factors: [f64; 3],
    tag: &str,
) -> Result<()> {
    let [ny, nx, nz] = dims;
    if ny == 0 || nx == 0 || nz == 0 {
        return Err("Volume dimensions must be positive".into());
    }
    let voxels = ny
        .checked_mul(nx)
        .and_then(|n| n.checked_mul(nz))
        .ok_or("Volume size overflow")?;
    if voxels > MAX_VOLUME_VOXELS {
        return Err(format!("Volume exceeds {MAX_VOLUME_VOXELS} voxel limit"));
    }
    if data.len() != voxels {
        return Err("Volume data does not match its dimensions".into());
    }
    if colormap.is_empty() || colormap.len() > MAX_COLORMAP_ROWS {
        return Err("Colormap must have 1..4096 rows".into());
    }
    if !colormap
        .iter()
        .flatten()
        .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
    {
        return Err("Colormap values must be finite and in [0, 1]".into());
    }
    if let Some([lo, hi]) = display_range
        && (!lo.is_finite() || !hi.is_finite() || lo >= hi)
    {
        return Err("DisplayRange must be finite [low high] with low < high".into());
    }
    if !scale_factors.iter().all(|v| v.is_finite() && *v > 0.0) {
        return Err("ScaleFactors must be a 3-element vector of positive finite numbers".into());
    }
    if tag.len() > 4096 {
        return Err("Tag exceeds 4096 bytes".into());
    }
    Ok(())
}

/// The property reports the data extrema in auto mode. Only the renderer
/// needs a nonzero interval when both extrema coincide.
fn slice_clim(display_range: Option<[f64; 2]>, data_limits: [f64; 2]) -> [f64; 2] {
    display_range.unwrap_or_else(|| super::automatic_limits(data_limits, Default::default()))
}

fn sync_slice_image(
    axes: &mut Axes,
    id: &ObjectIdentity,
    mut image: ImageObject,
    colormap: &Arc<Vec<[f64; 3]>>,
    clim: [f64; 2],
) {
    image.identity = id.clone();
    if let Some(owned) = axes
        .extras
        .images
        .iter_mut()
        .find(|i| i.identity.same_object(id))
    {
        *owned = image;
    } else {
        axes.extras.images.push(image);
        axes.extras.colormap = Some(colormap.clone());
        axes.extras.clim = Some(clim);
        axes.extras.y_reversed = true;
        axes.extras.equal_aspect = true;
        axes.extras.axis_off = true;
    }
}

fn sync_crosshair(axes: &mut Axes, id: &ObjectIdentity, points: Vec<[f64; 2]>, visible: bool) {
    if let Some(line) = axes.series.iter_mut().find(|s| s.identity.same_object(id)) {
        line.points = points.into();
        line.visible = visible;
    } else {
        let mut line = Series::new(points, PlotKind::Line);
        line.identity = id.clone();
        line.color = CROSSHAIR_COLOR;
        line.line_width = Some(1.5);
        line.visible = visible;
        axes.series.push(line);
    }
}

/// Model for `sliceViewer(V, ...)`.
#[derive(Clone, Debug)]
pub struct SliceViewerObject {
    pub identity: ObjectIdentity,
    pub axes_identity: ObjectIdentity,
    image_identity: ObjectIdentity,
    /// `[rows, cols, depth]` = `[ny, nx, nz]`, column-major `y + ny * (x + nx * z)`.
    pub dims: [usize; 3],
    pub data: Arc<[f64]>,
    pub data_class: ImageDataClass,
    pub slice_number: usize,
    pub slice_direction: SliceDirection,
    pub colormap: Arc<Vec<[f64; 3]>>,
    pub display_range: Option<[f64; 2]>,
    pub data_limits: [f64; 2],
    pub scale_factors: [f64; 3],
    pub visible: bool,
    pub tag: String,
}

impl SliceViewerObject {
    pub fn new(
        dims: [usize; 3],
        data: Arc<[f64]>,
        data_class: ImageDataClass,
        axes_identity: ObjectIdentity,
    ) -> Self {
        let (min_v, max_v) = finite_min_max(&data);
        let direction = SliceDirection::Z;
        Self {
            identity: ObjectIdentity::default(),
            axes_identity,
            image_identity: Default::default(),
            dims,
            data,
            data_class,
            slice_number: direction.default_slice(dims),
            slice_direction: direction,
            colormap: Arc::new(default_colormap()),
            display_range: None,
            data_limits: [min_v, max_v],
            scale_factors: [1.0, 1.0, 1.0],
            visible: true,
            tag: String::new(),
        }
    }

    pub fn max_slice(&self) -> usize {
        self.slice_direction.axis_len(self.dims)
    }

    /// Construct a handle for this model in its owning figure.
    pub fn graphics_handle(&self, figure: &ObjectIdentity) -> GraphicsHandle {
        GraphicsHandle::figure_object(figure, &self.identity, GraphicsKind::SliceViewer)
    }

    pub fn effective_display_range(&self) -> [f64; 2] {
        self.display_range.unwrap_or(self.data_limits)
    }

    pub fn sync_axes(&self, axes: &mut Axes) {
        let image = extract_slice_image(
            self.dims,
            &self.data,
            self.data_class,
            self.slice_direction,
            self.slice_number,
            self.scale_factors,
            self.visible,
        );
        sync_slice_image(
            axes,
            &self.image_identity,
            image,
            &self.colormap,
            self.clim(),
        );
    }

    pub(crate) fn clim(&self) -> [f64; 2] {
        slice_clim(self.display_range, self.data_limits)
    }

    pub(crate) fn image_id(&self) -> &ObjectIdentity {
        &self.image_identity
    }

    pub fn validate(&self) -> Result<()> {
        validate_volume_core(
            self.dims,
            &self.data,
            &self.colormap,
            self.display_range,
            self.scale_factors,
            &self.tag,
        )?;
        let max_n = self.max_slice();
        if self.slice_number == 0 || self.slice_number > max_n {
            return Err(format!(
                "SliceNumber must be an integer between 1 and {max_n}"
            ));
        }
        Ok(())
    }

    pub fn same_metadata(&self, other: &Self) -> bool {
        self.identity.same_object(&other.identity)
            && self.axes_identity.same_object(&other.axes_identity)
            && Arc::ptr_eq(&self.data, &other.data)
            && self.dims == other.dims
            && self.slice_number == other.slice_number
            && self.slice_direction == other.slice_direction
            && self.colormap.as_ref() == other.colormap.as_ref()
            && self.display_range == other.display_range
            && self.data_limits == other.data_limits
            && self.scale_factors == other.scale_factors
            && self.visible == other.visible
            && self.tag == other.tag
    }
}

/// Model for `orthosliceViewer(V, ...)`.
#[derive(Clone, Debug)]
pub struct OrthosliceViewerObject {
    pub identity: ObjectIdentity,
    /// `[XY, YZ, XZ]` axes identities.
    pub axes_identities: [ObjectIdentity; 3],
    image_identities: [ObjectIdentity; 3],
    crosshair_identities: [[ObjectIdentity; 2]; 3],
    /// `[rows, cols, depth]` = `[ny, nx, nz]`.
    pub dims: [usize; 3],
    pub data: Arc<[f64]>,
    pub data_class: ImageDataClass,
    /// 1-based `[x, y, z]` slice indices.
    pub slice_numbers: [usize; 3],
    pub crosshair_enabled: bool,
    pub colormap: Arc<Vec<[f64; 3]>>,
    pub display_range: Option<[f64; 2]>,
    pub data_limits: [f64; 2],
    pub scale_factors: [f64; 3],
    pub visible: bool,
    pub tag: String,
}

impl OrthosliceViewerObject {
    pub fn new(
        dims: [usize; 3],
        data: Arc<[f64]>,
        data_class: ImageDataClass,
        axes_identities: [ObjectIdentity; 3],
    ) -> Self {
        let [ny, nx, nz] = dims;
        let (min_v, max_v) = finite_min_max(&data);
        Self {
            identity: ObjectIdentity::default(),
            axes_identities,
            image_identities: Default::default(),
            crosshair_identities: Default::default(),
            dims,
            data,
            data_class,
            slice_numbers: [nx.div_ceil(2), ny.div_ceil(2), nz.div_ceil(2)],
            crosshair_enabled: true,
            colormap: Arc::new(default_colormap()),
            display_range: None,
            data_limits: [min_v, max_v],
            scale_factors: [1.0, 1.0, 1.0],
            visible: true,
            tag: String::new(),
        }
    }

    pub fn effective_display_range(&self) -> [f64; 2] {
        self.display_range.unwrap_or(self.data_limits)
    }

    /// Construct a handle for this model in its owning figure.
    pub fn graphics_handle(&self, figure: &ObjectIdentity) -> GraphicsHandle {
        GraphicsHandle::figure_object(figure, &self.identity, GraphicsKind::OrthosliceViewer)
    }

    pub fn sync_pane(&self, pane_idx: usize, axes: &mut Axes) {
        let [sx, sy, sz] = self.scale_factors;
        let [x, y, z] = self.slice_numbers;
        let (dir, slice, cx, cy) = match pane_idx {
            0 => (SliceDirection::Z, z, x as f64 * sx, y as f64 * sy),
            1 => (SliceDirection::X, x, z as f64 * sz, y as f64 * sy),
            2 => (SliceDirection::Y, y, x as f64 * sx, z as f64 * sz),
            _ => return,
        };
        let image = extract_slice_image(
            self.dims,
            &self.data,
            self.data_class,
            dir,
            slice,
            self.scale_factors,
            self.visible,
        );
        let (ex, ey) = image.extent();
        sync_slice_image(
            axes,
            &self.image_identities[pane_idx],
            image,
            &self.colormap,
            self.clim(),
        );
        let [vertical, horizontal] = &self.crosshair_identities[pane_idx];
        let visible = self.crosshair_enabled && self.visible;
        sync_crosshair(axes, vertical, vec![[cx, ey[0]], [cx, ey[1]]], visible);
        sync_crosshair(axes, horizontal, vec![[ex[0], cy], [ex[1], cy]], visible);
    }

    pub(crate) fn clim(&self) -> [f64; 2] {
        slice_clim(self.display_range, self.data_limits)
    }

    pub(crate) fn pane_graphics(&self, pane: usize) -> (&ObjectIdentity, &[ObjectIdentity]) {
        (
            &self.image_identities[pane],
            &self.crosshair_identities[pane],
        )
    }

    pub fn validate(&self) -> Result<()> {
        validate_volume_core(
            self.dims,
            &self.data,
            &self.colormap,
            self.display_range,
            self.scale_factors,
            &self.tag,
        )?;
        let [ny, nx, nz] = self.dims;
        let [x, y, z] = self.slice_numbers;
        if x == 0 || x > nx || y == 0 || y > ny || z == 0 || z > nz {
            return Err(format!(
                "SliceNumbers must be integers within [1..{nx}, 1..{ny}, 1..{nz}]"
            ));
        }
        Ok(())
    }

    pub fn same_metadata(&self, other: &Self) -> bool {
        self.identity.same_object(&other.identity)
            && self
                .axes_identities
                .iter()
                .zip(&other.axes_identities)
                .all(|(a, b)| a.same_object(b))
            && Arc::ptr_eq(&self.data, &other.data)
            && self.dims == other.dims
            && self.slice_numbers == other.slice_numbers
            && self.crosshair_enabled == other.crosshair_enabled
            && self.colormap.as_ref() == other.colormap.as_ref()
            && self.display_range == other.display_range
            && self.data_limits == other.data_limits
            && self.scale_factors == other.scale_factors
            && self.visible == other.visible
            && self.tag == other.tag
    }
}
