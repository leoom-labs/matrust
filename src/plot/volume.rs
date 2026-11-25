//! 3-D volume objects (`volshow`) and CPU orthographic raycaster.
use super::{ImageDataClass, colormap, view3d};
use crate::{NumericClass, ObjectIdentity, Result, TypedArray};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

pub const MAX_VOLUMES: usize = 16;
pub const MAX_VOLUME_VOXELS: usize = 64_000_000;
pub const DEFAULT_ALPHAMAP_LEN: usize = 256;
pub const DEFAULT_ISOSURFACE_COLOR: [f64; 3] = [0.7, 0.7, 0.7];
pub const DEFAULT_OVERLAY_ALPHA: f64 = 0.5;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OverlayRenderingStyle {
    #[default]
    LabelOverlay,
    VolumeOverlay,
}

impl OverlayRenderingStyle {
    pub fn parse(text: &str) -> Result<Self> {
        match text.to_ascii_lowercase().as_str() {
            "labeloverlay" => Ok(Self::LabelOverlay),
            "volumeoverlay" => Ok(Self::VolumeOverlay),
            _ => Err("OverlayRenderingStyle must be 'LabelOverlay' or 'VolumeOverlay'".into()),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::LabelOverlay => "LabelOverlay",
            Self::VolumeOverlay => "VolumeOverlay",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OverlayDataClass {
    Double,
    Logical,
    Typed(crate::NumericClass),
    Categorical {
        categories: Arc<Vec<String>>,
        ordinal: bool,
    },
}

#[derive(Clone, Debug)]
pub struct OverlayVolume {
    pub data: Arc<[f64]>,
    pub class: OverlayDataClass,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RenderingStyle {
    #[default]
    VolumeRendering,
    MaximumIntensityProjection,
    MinimumIntensityProjection,
    Isosurface,
    SlicePlanes,
}

impl RenderingStyle {
    pub fn parse(text: &str) -> Result<Self> {
        match text.to_ascii_lowercase().as_str() {
            "volumerendering" => Ok(Self::VolumeRendering),
            "maximumintensityprojection" => Ok(Self::MaximumIntensityProjection),
            "minimumintensityprojection" => Ok(Self::MinimumIntensityProjection),
            "isosurface" => Ok(Self::Isosurface),
            "sliceplanes" => Ok(Self::SlicePlanes),
            _ => Err("RenderingStyle must be 'VolumeRendering', 'MaximumIntensityProjection', 'MinimumIntensityProjection', 'Isosurface', or 'SlicePlanes'".into()),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::VolumeRendering => "VolumeRendering",
            Self::MaximumIntensityProjection => "MaximumIntensityProjection",
            Self::MinimumIntensityProjection => "MinimumIntensityProjection",
            Self::Isosurface => "Isosurface",
            Self::SlicePlanes => "SlicePlanes",
        }
    }
}

pub fn default_alphamap(len: usize) -> Vec<f64> {
    if len <= 1 {
        return vec![1.0; len];
    }
    (0..len).map(|i| i as f64 / (len - 1) as f64).collect()
}

pub fn default_colormap() -> Vec<[f64; 3]> {
    colormap::named("gray", colormap::DEFAULT_ROWS)
        .ok()
        .flatten()
        .unwrap_or_else(|| vec![[0., 0., 0.], [1., 1., 1.]])
}

/// Fixed deterministic 256-color distinct label palette.
/// MATLAB's default label palette is not published under terms we may reproduce,
/// so we use 7 high-contrast primary/secondary colors followed by golden-ratio
/// HSV stepping so adjacent label indices remain visually distinct.
pub fn default_overlay_colormap() -> Vec<[f64; 3]> {
    const BASE: [[f64; 3]; 7] = [
        [0.0, 0.447, 0.741],
        [0.850, 0.325, 0.098],
        [0.929, 0.694, 0.125],
        [0.494, 0.184, 0.556],
        [0.466, 0.674, 0.188],
        [0.301, 0.745, 0.933],
        [0.635, 0.078, 0.184],
    ];
    let mut table = Vec::with_capacity(DEFAULT_ALPHAMAP_LEN);
    table.extend_from_slice(&BASE);
    for i in BASE.len()..DEFAULT_ALPHAMAP_LEN {
        let h = ((i as f64) * 0.6180339887498949).fract();
        let s = 0.75 + 0.25 * ((i % 3) as f64 / 2.0);
        let v = 0.7 + 0.3 * (((i / 3) % 2) as f64);
        let h6 = h * 6.0;
        let sector = h6.floor() as usize % 6;
        let f = h6 - h6.floor();
        let p = v * (1.0 - s);
        let q = v * (1.0 - s * f);
        let t = v * (1.0 - s * (1.0 - f));
        let rgb = match sector {
            0 => [v, t, p],
            1 => [q, v, p],
            2 => [p, v, t],
            3 => [p, q, v],
            4 => [t, p, v],
            _ => [v, p, q],
        };
        table.push(rgb);
    }
    table
}

pub fn default_overlay_alphamap() -> Vec<f64> {
    vec![DEFAULT_OVERLAY_ALPHA]
}

#[derive(Clone, Debug)]
pub struct VolumeObject {
    pub identity: ObjectIdentity,
    pub(crate) stack_order: ObjectIdentity,
    pub(crate) data_revision: ObjectIdentity,
    /// `[rows, cols, depth]` = `[ny, nx, nz]`, column-major `y + ny * (x + nx * z)`.
    pub dims: [usize; 3],
    pub data: Arc<[f64]>,
    pub data_class: ImageDataClass,
    /// Original `int64` or `uint64` storage, shared with the input array.
    /// Legacy volumes may have only the floating-point render samples.
    pub(crate) exact_data: Option<TypedArray>,
    pub rendering_style: RenderingStyle,
    pub colormap: Arc<Vec<[f64; 3]>>,
    pub alphamap: Arc<Vec<f64>>,
    pub data_limits: [f64; 2],
    pub data_limits_manual: bool,
    pub isosurface_value: f64,
    pub isosurface_color: [f64; 3],
    pub slice_plane_values: Vec<[f64; 4]>,
    /// Voxel spacing scale `[sx, sy, sz]`.
    pub transformation: [f64; 3],
    pub visible: bool,
    pub overlay_data: Option<OverlayVolume>,
    pub overlay_rendering_style: OverlayRenderingStyle,
    pub overlay_colormap: Arc<Vec<[f64; 3]>>,
    pub overlay_alphamap: Arc<Vec<f64>>,
    pub overlay_display_range: [f64; 2],
    pub overlay_display_range_manual: bool,
    /// Display-space projected bounds populated by `view3d::Projection::display`.
    pub(crate) projected_bounds: Option<([f64; 2], [f64; 2])>,
    pub display_view: Option<[f64; 2]>,
}

impl VolumeObject {
    pub fn new(dims: [usize; 3], data: Arc<[f64]>, data_class: ImageDataClass) -> Self {
        let (min_v, max_v) = finite_min_max(&data);
        let data_limits = [min_v, max_v];
        let isosurface_value = 0.5 * (min_v + max_v);
        let [ny, nx, nz] = dims;
        let cx = (nx as f64 + 1.0) * 0.5;
        let cy = (ny as f64 + 1.0) * 0.5;
        let cz = (nz as f64 + 1.0) * 0.5;
        let identity = ObjectIdentity::default();
        Self {
            stack_order: identity.clone(),
            data_revision: identity.clone(),
            identity,
            dims,
            data,
            data_class,
            exact_data: None,
            rendering_style: RenderingStyle::VolumeRendering,
            colormap: Arc::new(default_colormap()),
            alphamap: Arc::new(default_alphamap(DEFAULT_ALPHAMAP_LEN)),
            data_limits,
            data_limits_manual: false,
            isosurface_value,
            isosurface_color: DEFAULT_ISOSURFACE_COLOR,
            slice_plane_values: vec![
                [1.0, 0.0, 0.0, -cx],
                [0.0, 1.0, 0.0, -cy],
                [0.0, 0.0, 1.0, -cz],
            ],
            transformation: [1.0, 1.0, 1.0],
            visible: true,
            overlay_data: None,
            overlay_rendering_style: OverlayRenderingStyle::LabelOverlay,
            overlay_colormap: Arc::new(default_overlay_colormap()),
            overlay_alphamap: Arc::new(default_overlay_alphamap()),
            overlay_display_range: [0.0, 1.0],
            overlay_display_range_manual: false,
            projected_bounds: None,
            display_view: None,
        }
    }

    /// Storage revision used by raster caches without retaining the voxel buffer.
    pub fn data_revision(&self) -> &ObjectIdentity {
        &self.data_revision
    }

    pub fn validate(&self) -> Result<()> {
        let [ny, nx, nz] = self.dims;
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
        if self.data.len() != voxels {
            return Err("Volume data does not match its dimensions".into());
        }
        if let Some(exact) = &self.exact_data {
            let class = exact.class();
            if !matches!(class, NumericClass::Int64 | NumericClass::Uint64)
                || self.data_class != ImageDataClass::Typed(class)
                || exact.dims().as_ref() != self.dims
            {
                return Err("Volume exact data must match the Data class and size".into());
            }
        }
        if self.colormap.is_empty() || self.colormap.len() > super::MAX_COLORMAP_ROWS {
            return Err("Volume Colormap must have 1..4096 rows".into());
        }
        if !self
            .colormap
            .iter()
            .flatten()
            .all(|v| v.is_finite() && (0. ..=1.).contains(v))
        {
            return Err("Volume Colormap values must be in [0, 1]".into());
        }
        if self.alphamap.is_empty() || self.alphamap.len() > super::MAX_COLORMAP_ROWS {
            return Err("Volume Alphamap must have 1..4096 elements".into());
        }
        if !self
            .alphamap
            .iter()
            .all(|v| v.is_finite() && (0. ..=1.).contains(v))
        {
            return Err("Volume Alphamap values must be in [0, 1]".into());
        }
        if !self.data_limits.iter().all(|v| v.is_finite())
            || self.data_limits[0] > self.data_limits[1]
        {
            return Err("Volume DataLimits must be finite [min max] with min <= max".into());
        }
        if !self.isosurface_value.is_finite() {
            return Err("Volume IsosurfaceValue must be finite".into());
        }
        if !self
            .isosurface_color
            .iter()
            .all(|v| v.is_finite() && (0. ..=1.).contains(v))
        {
            return Err("Volume IsosurfaceColor values must be in [0, 1]".into());
        }
        if !self
            .slice_plane_values
            .iter()
            .flatten()
            .all(|v| v.is_finite())
        {
            return Err("Volume SlicePlaneValues must be finite".into());
        }
        if !self
            .transformation
            .iter()
            .all(|v| v.is_finite() && *v > 0.0)
        {
            return Err("Volume Transformation scale must be finite and positive".into());
        }
        if let Some(overlay) = &self.overlay_data
            && overlay.data.len() != voxels
        {
            return Err("Volume OverlayData size must match Data size".into());
        }
        if self.overlay_colormap.is_empty()
            || self.overlay_colormap.len() > super::MAX_COLORMAP_ROWS
        {
            return Err("Volume OverlayColormap must have 1..4096 rows".into());
        }
        if !self
            .overlay_colormap
            .iter()
            .flatten()
            .all(|v| v.is_finite() && (0. ..=1.).contains(v))
        {
            return Err("Volume OverlayColormap values must be in [0, 1]".into());
        }
        if self.overlay_alphamap.is_empty()
            || self.overlay_alphamap.len() > super::MAX_COLORMAP_ROWS
        {
            return Err("Volume OverlayAlphamap must have 1..4096 elements".into());
        }
        if !self
            .overlay_alphamap
            .iter()
            .all(|v| v.is_finite() && (0. ..=1.).contains(v))
        {
            return Err("Volume OverlayAlphamap values must be in [0, 1]".into());
        }
        if !self.overlay_display_range.iter().all(|v| v.is_finite())
            || self.overlay_display_range[0] > self.overlay_display_range[1]
        {
            return Err(
                "Volume OverlayDisplayRange must be finite [min max] with min <= max".into(),
            );
        }
        Ok(())
    }

    pub fn same_metadata(&self, other: &Self) -> bool {
        let same_overlay = match (&self.overlay_data, &other.overlay_data) {
            (None, None) => true,
            (Some(a), Some(b)) => Arc::ptr_eq(&a.data, &b.data) && a.class == b.class,
            _ => false,
        };
        self.identity.same_object(&other.identity)
            && self.stack_order.same_object(&other.stack_order)
            && Arc::ptr_eq(&self.data, &other.data)
            && self.dims == other.dims
            && self.data_class == other.data_class
            && self.rendering_style == other.rendering_style
            && *self.colormap == *other.colormap
            && *self.alphamap == *other.alphamap
            && self.data_limits == other.data_limits
            && self.data_limits_manual == other.data_limits_manual
            && self.isosurface_value == other.isosurface_value
            && self.isosurface_color == other.isosurface_color
            && self.slice_plane_values == other.slice_plane_values
            && self.transformation == other.transformation
            && self.visible == other.visible
            && same_overlay
            && self.overlay_rendering_style == other.overlay_rendering_style
            && *self.overlay_colormap == *other.overlay_colormap
            && *self.overlay_alphamap == *other.overlay_alphamap
            && self.overlay_display_range == other.overlay_display_range
            && self.overlay_display_range_manual == other.overlay_display_range_manual
    }

    /// Data-space `[x_range, y_range, z_range]` of the volume box.
    pub fn extent(&self) -> ([f64; 2], [f64; 2], [f64; 2]) {
        let [ny, nx, nz] = self.dims;
        let [sx, sy, sz] = self.transformation;
        (
            [0.5 * sx, (nx as f64 + 0.5) * sx],
            [0.5 * sy, (ny as f64 + 0.5) * sy],
            [0.5 * sz, (nz as f64 + 0.5) * sz],
        )
    }

    /// In a top view, raster row one has the lowest data Y coordinate.
    pub fn display_flip_y(&self, axes: &super::Axes) -> bool {
        self.display_view.unwrap_or(axes.view) == view3d::VIEW_2D && !axes.extras.y_reversed
    }
}

pub(crate) fn finite_min_max(data: &[f64]) -> (f64, f64) {
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for &v in data {
        if v.is_finite() {
            lo = lo.min(v);
            hi = hi.max(v);
        }
    }
    if lo <= hi { (lo, hi) } else { (0.0, 1.0) }
}

#[inline]
fn normalize(val: f64, limits: [f64; 2]) -> f64 {
    let span = limits[1] - limits[0];
    if span <= 0.0 || !val.is_finite() {
        0.0
    } else {
        ((val - limits[0]) / span).clamp(0.0, 1.0)
    }
}

#[inline]
fn map_round_rgb(map: &[[f64; 3]], t: f64) -> [u8; 3] {
    if map.is_empty() {
        return [0, 0, 0];
    }
    let idx = if map.len() == 1 || !t.is_finite() {
        0
    } else {
        ((t.clamp(0.0, 1.0) * (map.len() - 1) as f64).round() as usize).min(map.len() - 1)
    };
    map[idx].map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8)
}

#[inline]
fn sample_alpha(alphamap: &[f64], t: f64) -> f64 {
    if alphamap.is_empty() || !t.is_finite() {
        return 0.0;
    }
    if alphamap.len() == 1 {
        return alphamap[0].clamp(0.0, 1.0);
    }
    let x = t.clamp(0.0, 1.0) * (alphamap.len() - 1) as f64;
    let i = (x.floor() as usize).min(alphamap.len() - 2);
    let f = x - i as f64;
    (alphamap[i] + (alphamap[i + 1] - alphamap[i]) * f).clamp(0.0, 1.0)
}

#[inline]
fn sample_trilinear(data: &[f64], dims: [usize; 3], fx: f64, fy: f64, fz: f64) -> Option<f64> {
    let [ny, nx, nz] = dims;
    let x = fx.clamp(0.0, (nx - 1) as f64);
    let y = fy.clamp(0.0, (ny - 1) as f64);
    let z = fz.clamp(0.0, (nz - 1) as f64);
    let x0 = (x.floor() as usize).min(nx.saturating_sub(1));
    let y0 = (y.floor() as usize).min(ny.saturating_sub(1));
    let z0 = (z.floor() as usize).min(nz.saturating_sub(1));
    let x1 = (x0 + 1).min(nx - 1);
    let y1 = (y0 + 1).min(ny - 1);
    let z1 = (z0 + 1).min(nz - 1);
    let tx = x - x0 as f64;
    let ty = y - y0 as f64;
    let tz = z - z0 as f64;

    // Fast path for exact voxel center (or near-zero fractional part).
    if tx == 0.0 && ty == 0.0 && tz == 0.0 {
        let v = data[y0 + ny * (x0 + nx * z0)];
        return v.is_finite().then_some(v);
    }

    let c000 = data[y0 + ny * (x0 + nx * z0)];
    let c100 = data[y0 + ny * (x1 + nx * z0)];
    let c010 = data[y1 + ny * (x0 + nx * z0)];
    let c110 = data[y1 + ny * (x1 + nx * z0)];
    let c001 = data[y0 + ny * (x0 + nx * z1)];
    let c101 = data[y0 + ny * (x1 + nx * z1)];
    let c011 = data[y1 + ny * (x0 + nx * z1)];
    let c111 = data[y1 + ny * (x1 + nx * z1)];

    let c00 = c000 + (c100 - c000) * tx;
    let c10 = c010 + (c110 - c010) * tx;
    let c01 = c001 + (c101 - c001) * tx;
    let c11 = c011 + (c111 - c011) * tx;

    let c0 = c00 + (c10 - c00) * ty;
    let c1 = c01 + (c11 - c01) * ty;

    let v = c0 + (c1 - c0) * tz;
    v.is_finite().then_some(v)
}

#[inline]
fn intersect_box(origin: [f64; 3], dir: [f64; 3], half: [f64; 3]) -> Option<(f64, f64)> {
    let mut t_near = f64::NEG_INFINITY;
    let mut t_far = f64::INFINITY;
    for axis in 0..3 {
        if dir[axis].abs() < 1e-15 {
            if origin[axis] < -half[axis] || origin[axis] > half[axis] {
                return None;
            }
        } else {
            let inv = 1.0 / dir[axis];
            let mut t0 = (-half[axis] - origin[axis]) * inv;
            let mut t1 = (half[axis] - origin[axis]) * inv;
            if t0 > t1 {
                std::mem::swap(&mut t0, &mut t1);
            }
            t_near = t_near.max(t0);
            t_far = t_far.min(t1);
            if t_near > t_far {
                return None;
            }
        }
    }
    if t_far > t_near {
        Some((t_near, t_far))
    } else {
        None
    }
}

/// Headlight shading constants for `Isosurface` rendering (ambient + diffuse = 1.0).
const ISO_AMBIENT: f64 = 0.25;
const ISO_DIFFUSE: f64 = 0.75;
/// Early ray termination opacity threshold for `VolumeRendering`.
const VOLUME_EARLY_STOP_ALPHA: f64 = 0.99;

struct RayContext<'a> {
    volume: &'a VolumeObject,
    origin: [f64; 3],
    ray_dir: [f64; 3],
    half: [f64; 3],
    t_near: f64,
    t_far: f64,
    step: f64,
    max_samples: usize,
}

impl<'a> RayContext<'a> {
    #[inline]
    fn voxel_coords(&self, t: f64) -> [f64; 3] {
        let [sx, sy, sz] = self.volume.transformation;
        let px = self.origin[0] + t * self.ray_dir[0];
        let py = self.origin[1] + t * self.ray_dir[1];
        let pz = self.origin[2] + t * self.ray_dir[2];
        [
            (px + self.half[0]) / sx - 0.5,
            (self.half[1] - py) / sy - 0.5,
            (pz + self.half[2]) / sz - 0.5,
        ]
    }
}

/// Axis of a cardinal ray, or `None` when the ray is oblique.
///
/// `viewmtx` stores exact camera inputs (azimuth and elevation that are exact
/// multiples of 90 degrees, and any azimuth at elevation ±90) as exact `0` and
/// `±1`. Signed zero compares equal to zero. A direction tolerance promotes
/// near-axis oblique rays, such as elevation `89.99999999999`, onto the exact
/// voxel walk.
#[inline]
fn axis_aligned_ray_axis(ray_dir: [f64; 3]) -> Option<usize> {
    let mut aligned = None;
    for (axis, component) in ray_dir.iter().copied().enumerate() {
        if component == 0.0 {
            continue;
        }
        if aligned.is_some() || component.abs() != 1.0 {
            return None;
        }
        aligned = Some(axis);
    }
    aligned
}

#[inline]
fn snap_voxel_coord(f: f64, len: usize) -> f64 {
    let clamped = f.clamp(0.0, len.saturating_sub(1) as f64);
    let rounded = clamped.round();
    if (clamped - rounded).abs() <= 1e-12 {
        rounded
    } else {
        clamped
    }
}

fn render_mip_minip_ray(
    ray: &RayContext<'_>,
    is_max: bool,
    cancel: Option<&AtomicBool>,
) -> [u8; 4] {
    let mut best = if is_max {
        f64::NEG_INFINITY
    } else {
        f64::INFINITY
    };
    let mut hit = false;
    let mut update = |val: f64| {
        if !hit || if is_max { val > best } else { val < best } {
            best = val;
            hit = true;
        }
    };

    if let Some(axis) = axis_aligned_ray_axis(ray.ray_dir) {
        // Sample exact voxel centers along axis-aligned rays so non-unit
        // Transformation spacing never drifts between slices or columns.
        let [ny, nx, nz] = ray.volume.dims;
        let [fx0, fy0, fz0] = ray.voxel_coords(0.5 * (ray.t_near + ray.t_far));
        let mut fx = snap_voxel_coord(fx0, nx);
        let mut fy = snap_voxel_coord(fy0, ny);
        let mut fz = snap_voxel_coord(fz0, nz);
        let (len, forward) = match axis {
            0 => (nx, ray.ray_dir[0] > 0.0),
            1 => (ny, ray.ray_dir[1] < 0.0),
            _ => (nz, ray.ray_dir[2] > 0.0),
        };
        for step_idx in 0..len {
            if step_idx & 4095 == 0
                && cancel.is_some_and(|c| c.load(std::sync::atomic::Ordering::Relaxed))
            {
                return [0, 0, 0, 0];
            }
            let idx = if forward {
                step_idx
            } else {
                len - 1 - step_idx
            };
            match axis {
                0 => fx = idx as f64,
                1 => fy = idx as f64,
                _ => fz = idx as f64,
            }
            if let Some(val) = sample_trilinear(&ray.volume.data, ray.volume.dims, fx, fy, fz) {
                update(val);
            }
        }
    } else {
        let mut t = ray.t_near + 0.5 * ray.step;
        let mut samples = 0usize;
        // Oblique MIP/MinIP keeps its original 8192-sample ceiling.
        // `ray.max_samples` rises to 65536 for finer steps of the other styles.
        while t < ray.t_far && samples < 8192 {
            samples += 1;
            let [fx, fy, fz] = ray.voxel_coords(t);
            if let Some(val) = sample_trilinear(&ray.volume.data, ray.volume.dims, fx, fy, fz) {
                update(val);
            }
            let next = t + ray.step;
            if next <= t {
                break;
            }
            t = next;
        }
    }

    if hit {
        let norm = normalize(best, ray.volume.data_limits);
        let [r, g, b] = map_round_rgb(&ray.volume.colormap, norm);
        [r, g, b, 255]
    } else {
        [0, 0, 0, 0]
    }
}

fn render_isosurface_ray(ray: &RayContext<'_>) -> [u8; 4] {
    let limits = ray.volume.data_limits;
    let span = limits[1] - limits[0];
    let iso_norm = if span > 0.0 {
        (ray.volume.isosurface_value - limits[0]) / span
    } else {
        0.0
    };
    let signed_dist = |val: f64| -> f64 {
        if span > 0.0 {
            (val - limits[0]) / span - iso_norm
        } else {
            val - ray.volume.isosurface_value
        }
    };

    let data = &ray.volume.data;
    let dims = ray.volume.dims;
    let [sx, sy, sz] = ray.volume.transformation;

    let mut t_prev = ray.t_near;
    let mut d_prev = {
        let [fx, fy, fz] = ray.voxel_coords(t_prev);
        sample_trilinear(data, dims, fx, fy, fz).map(signed_dist)
    };
    let mut samples = 0usize;
    let mut t_hit: Option<f64> = None;

    while t_prev < ray.t_far && samples < ray.max_samples {
        samples += 1;
        let next = (t_prev + ray.step).min(ray.t_far);
        if next <= t_prev {
            break;
        }
        let [fx, fy, fz] = ray.voxel_coords(next);
        let d_curr = sample_trilinear(data, dims, fx, fy, fz).map(signed_dist);
        if let (Some(dp), Some(dc)) = (d_prev, d_curr)
            && dp * dc <= 0.0
        {
            let frac = if (dc - dp).abs() > 1e-15 {
                (-dp / (dc - dp)).clamp(0.0, 1.0)
            } else {
                0.0
            };
            t_hit = Some(t_prev + frac * (next - t_prev));
            break;
        }
        t_prev = next;
        d_prev = d_curr;
    }

    let Some(t_hit) = t_hit else {
        return [0, 0, 0, 0];
    };

    let [fx, fy, fz] = ray.voxel_coords(t_hit);
    let h = 0.5;
    let gx = match (
        sample_trilinear(data, dims, fx + h, fy, fz),
        sample_trilinear(data, dims, fx - h, fy, fz),
    ) {
        (Some(vp), Some(vm)) => (vp - vm) / (2.0 * h * sx),
        _ => 0.0,
    };
    let gy = match (
        sample_trilinear(data, dims, fx, fy + h, fz),
        sample_trilinear(data, dims, fx, fy - h, fz),
    ) {
        (Some(vp), Some(vm)) => -(vp - vm) / (2.0 * h * sy),
        _ => 0.0,
    };
    let gz = match (
        sample_trilinear(data, dims, fx, fy, fz + h),
        sample_trilinear(data, dims, fx, fy, fz - h),
    ) {
        (Some(vp), Some(vm)) => (vp - vm) / (2.0 * h * sz),
        _ => 0.0,
    };
    let glen = (gx * gx + gy * gy + gz * gz).sqrt();
    let n_dot_l = if glen > 1e-12 {
        // Headlight points along -ray_dir; two-sided |N . L|.
        ((gx * (-ray.ray_dir[0]) + gy * (-ray.ray_dir[1]) + gz * (-ray.ray_dir[2])) / glen).abs()
    } else {
        1.0
    };
    let shade = ISO_AMBIENT + ISO_DIFFUSE * n_dot_l;
    let c = ray.volume.isosurface_color;
    [
        ((c[0] * shade).clamp(0.0, 1.0) * 255.0).round() as u8,
        ((c[1] * shade).clamp(0.0, 1.0) * 255.0).round() as u8,
        ((c[2] * shade).clamp(0.0, 1.0) * 255.0).round() as u8,
        255,
    ]
}

fn render_slice_planes_ray(ray: &RayContext<'_>) -> [u8; 4] {
    let [sx, sy, sz] = ray.volume.transformation;
    let v0 = [
        (ray.origin[0] + ray.half[0]) / sx + 0.5,
        (ray.half[1] - ray.origin[1]) / sy + 0.5,
        (ray.origin[2] + ray.half[2]) / sz + 0.5,
    ];
    let dv = [
        ray.ray_dir[0] / sx,
        -ray.ray_dir[1] / sy,
        ray.ray_dir[2] / sz,
    ];
    let mut best_t = f64::INFINITY;
    let mut best_val = None;
    for &[a, b, c, d] in &ray.volume.slice_plane_values {
        let denom = a * dv[0] + b * dv[1] + c * dv[2];
        if denom.abs() <= 1e-12 {
            continue;
        }
        let t_hit = -(a * v0[0] + b * v0[1] + c * v0[2] + d) / denom;
        if t_hit < ray.t_near - 1e-9 || t_hit > ray.t_far + 1e-9 {
            continue;
        }
        if t_hit < best_t {
            let fx = v0[0] + t_hit * dv[0] - 1.0;
            let fy = v0[1] + t_hit * dv[1] - 1.0;
            let fz = v0[2] + t_hit * dv[2] - 1.0;
            if let Some(val) = sample_trilinear(&ray.volume.data, ray.volume.dims, fx, fy, fz) {
                best_t = t_hit;
                best_val = Some(val);
            }
        }
    }
    if let Some(val) = best_val {
        let norm = normalize(val, ray.volume.data_limits);
        let [r, g, b] = map_round_rgb(&ray.volume.colormap, norm);
        [r, g, b, 255]
    } else {
        [0, 0, 0, 0]
    }
}

fn render_volume_ray(ray: &RayContext<'_>) -> [u8; 4] {
    let [sx, sy, sz] = ray.volume.transformation;
    // Reference sample step is 1.0 in voxel units.
    let voxel_per_world = ((ray.ray_dir[0] / sx).powi(2)
        + (ray.ray_dir[1] / sy).powi(2)
        + (ray.ray_dir[2] / sz).powi(2))
    .sqrt();
    let mut acc_rgb = [0.0f64; 3];
    let mut acc_a = 0.0f64;
    let mut t_curr = ray.t_near;
    let mut samples = 0usize;
    while t_curr < ray.t_far && samples < ray.max_samples {
        samples += 1;
        let t_next = (t_curr + ray.step).min(ray.t_far);
        let dt = t_next - t_curr;
        if dt <= 0.0 {
            break;
        }
        let t_mid = 0.5 * (t_curr + t_next);
        let [fx, fy, fz] = ray.voxel_coords(t_mid);
        if let Some(val) = sample_trilinear(&ray.volume.data, ray.volume.dims, fx, fy, fz) {
            let norm = normalize(val, ray.volume.data_limits);
            let a_raw = sample_alpha(&ray.volume.alphamap, norm);
            if a_raw > 0.0 {
                let a = 1.0 - (1.0 - a_raw).powf(dt * voxel_per_world);
                let c = colormap::sample(&ray.volume.colormap, norm);
                let weight = (1.0 - acc_a) * a;
                acc_rgb[0] += weight * c[0];
                acc_rgb[1] += weight * c[1];
                acc_rgb[2] += weight * c[2];
                acc_a += weight;
                if acc_a >= VOLUME_EARLY_STOP_ALPHA {
                    acc_a = 1.0;
                    break;
                }
            }
        }
        t_curr = t_next;
    }
    if acc_a > 0.0 {
        let inv_a = 1.0 / acc_a;
        [
            ((acc_rgb[0] * inv_a).clamp(0.0, 1.0) * 255.0).round() as u8,
            ((acc_rgb[1] * inv_a).clamp(0.0, 1.0) * 255.0).round() as u8,
            ((acc_rgb[2] * inv_a).clamp(0.0, 1.0) * 255.0).round() as u8,
            (acc_a.clamp(0.0, 1.0) * 255.0).round() as u8,
        ]
    } else {
        [0, 0, 0, 0]
    }
}

#[inline]
fn sample_nearest_label(
    data: &[f64],
    dims: [usize; 3],
    fx: f64,
    fy: f64,
    fz: f64,
) -> Option<usize> {
    let [ny, nx, nz] = dims;
    let ix = (fx.round() as isize).clamp(0, nx as isize - 1) as usize;
    let iy = (fy.round() as isize).clamp(0, ny as isize - 1) as usize;
    let iz = (fz.round() as isize).clamp(0, nz as isize - 1) as usize;
    let v = data[iy + ny * (ix + nx * iz)];
    if !v.is_finite() || v <= 0.0 {
        return None;
    }
    let k = v.round() as usize;
    (k > 0).then_some(k)
}

#[inline]
fn sample_overlay_at(
    volume: &VolumeObject,
    overlay: &OverlayVolume,
    fx: f64,
    fy: f64,
    fz: f64,
) -> Option<([f64; 3], f64)> {
    match volume.overlay_rendering_style {
        OverlayRenderingStyle::LabelOverlay => {
            let k = sample_nearest_label(&overlay.data, volume.dims, fx, fy, fz)?;
            let cmap = &volume.overlay_colormap;
            let amap = &volume.overlay_alphamap;
            let a_raw = if amap.len() == 1 {
                amap[0]
            } else {
                amap[(k - 1) % amap.len()]
            }
            .clamp(0.0, 1.0);
            if a_raw <= 0.0 {
                return None;
            }
            let c = cmap[(k - 1) % cmap.len()];
            Some((c, a_raw))
        }
        OverlayRenderingStyle::VolumeOverlay => {
            let val = sample_trilinear(&overlay.data, volume.dims, fx, fy, fz)?;
            let norm = normalize(val, volume.overlay_display_range);
            let a_raw = sample_alpha(&volume.overlay_alphamap, norm);
            if a_raw <= 0.0 {
                return None;
            }
            let c = colormap::sample(&volume.overlay_colormap, norm);
            Some((c, a_raw))
        }
    }
}

#[inline]
fn ray_voxel_per_world(ray: &RayContext<'_>) -> f64 {
    let [sx, sy, sz] = ray.volume.transformation;
    ((ray.ray_dir[0] / sx).powi(2) + (ray.ray_dir[1] / sy).powi(2) + (ray.ray_dir[2] / sz).powi(2))
        .sqrt()
}

fn composite_overlay_ray(ray: &RayContext<'_>, overlay: &OverlayVolume, base: [u8; 4]) -> [u8; 4] {
    let voxel_per_world = ray_voxel_per_world(ray);
    let mut ov_rgb = [0.0f64; 3];
    let mut ov_a = 0.0f64;
    let mut t_curr = ray.t_near;
    let mut samples = 0usize;
    while t_curr < ray.t_far && samples < ray.max_samples {
        samples += 1;
        let t_next = (t_curr + ray.step).min(ray.t_far);
        let dt = t_next - t_curr;
        if dt <= 0.0 {
            break;
        }
        let t_mid = 0.5 * (t_curr + t_next);
        let [fx, fy, fz] = ray.voxel_coords(t_mid);
        if let Some((oc, oa_raw)) = sample_overlay_at(ray.volume, overlay, fx, fy, fz) {
            let oa = 1.0 - (1.0 - oa_raw).powf(dt * voxel_per_world);
            let weight = (1.0 - ov_a) * oa;
            ov_rgb[0] += weight * oc[0];
            ov_rgb[1] += weight * oc[1];
            ov_rgb[2] += weight * oc[2];
            ov_a += weight;
            if ov_a >= VOLUME_EARLY_STOP_ALPHA {
                ov_a = 1.0;
                break;
            }
        }
        t_curr = t_next;
    }
    if ov_a <= 0.0 {
        return base;
    }
    let base_a = base[3] as f64 / 255.0;
    let out_a = (ov_a + (1.0 - ov_a) * base_a).clamp(0.0, 1.0);
    if out_a <= 0.0 {
        return [0, 0, 0, 0];
    }
    let inv_a = 1.0 / out_a;
    let base_scale = (1.0 - ov_a) * base_a / 255.0;
    [
        (((ov_rgb[0] + base_scale * base[0] as f64) * inv_a).clamp(0.0, 1.0) * 255.0).round() as u8,
        (((ov_rgb[1] + base_scale * base[1] as f64) * inv_a).clamp(0.0, 1.0) * 255.0).round() as u8,
        (((ov_rgb[2] + base_scale * base[2] as f64) * inv_a).clamp(0.0, 1.0) * 255.0).round() as u8,
        (out_a * 255.0).round() as u8,
    ]
}

fn render_volume_with_overlay_ray(ray: &RayContext<'_>, overlay: &OverlayVolume) -> [u8; 4] {
    let voxel_per_world = ray_voxel_per_world(ray);
    let mut acc_rgb = [0.0f64; 3];
    let mut acc_a = 0.0f64;
    let mut t_curr = ray.t_near;
    let mut samples = 0usize;
    while t_curr < ray.t_far && samples < ray.max_samples {
        samples += 1;
        let t_next = (t_curr + ray.step).min(ray.t_far);
        let dt = t_next - t_curr;
        if dt <= 0.0 {
            break;
        }
        let t_mid = 0.5 * (t_curr + t_next);
        let [fx, fy, fz] = ray.voxel_coords(t_mid);
        let base_sample =
            sample_trilinear(&ray.volume.data, ray.volume.dims, fx, fy, fz).and_then(|val| {
                let norm = normalize(val, ray.volume.data_limits);
                let a_raw = sample_alpha(&ray.volume.alphamap, norm);
                (a_raw > 0.0).then(|| {
                    let a = 1.0 - (1.0 - a_raw).powf(dt * voxel_per_world);
                    let c = colormap::sample(&ray.volume.colormap, norm);
                    (c, a)
                })
            });
        let ov_sample = sample_overlay_at(ray.volume, overlay, fx, fy, fz).map(|(oc, oa_raw)| {
            let oa = 1.0 - (1.0 - oa_raw).powf(dt * voxel_per_world);
            (oc, oa)
        });
        let (s_rgb, s_a) = match (base_sample, ov_sample) {
            (None, None) => {
                t_curr = t_next;
                continue;
            }
            (Some((bc, ba)), None) => ([ba * bc[0], ba * bc[1], ba * bc[2]], ba),
            (None, Some((oc, oa))) => ([oa * oc[0], oa * oc[1], oa * oc[2]], oa),
            (Some((bc, ba)), Some((oc, oa))) => {
                let rem = (1.0 - oa) * ba;
                (
                    [
                        oa * oc[0] + rem * bc[0],
                        oa * oc[1] + rem * bc[1],
                        oa * oc[2] + rem * bc[2],
                    ],
                    oa + rem,
                )
            }
        };
        let one_minus = 1.0 - acc_a;
        acc_rgb[0] += one_minus * s_rgb[0];
        acc_rgb[1] += one_minus * s_rgb[1];
        acc_rgb[2] += one_minus * s_rgb[2];
        acc_a += one_minus * s_a;
        if acc_a >= VOLUME_EARLY_STOP_ALPHA {
            acc_a = 1.0;
            break;
        }
        t_curr = t_next;
    }
    if acc_a > 0.0 {
        let inv_a = 1.0 / acc_a;
        [
            ((acc_rgb[0] * inv_a).clamp(0.0, 1.0) * 255.0).round() as u8,
            ((acc_rgb[1] * inv_a).clamp(0.0, 1.0) * 255.0).round() as u8,
            ((acc_rgb[2] * inv_a).clamp(0.0, 1.0) * 255.0).round() as u8,
            (acc_a.clamp(0.0, 1.0) * 255.0).round() as u8,
        ]
    } else {
        [0, 0, 0, 0]
    }
}

/// Orthographic CPU raycaster.
///
/// `size = None` renders at native resolution (one voxel per pixel for an
/// axis-aligned camera). Output is row-major top-to-bottom, left-to-right RGBA.
pub fn render_volume(
    volume: &VolumeObject,
    view: [f64; 2],
    size: Option<(usize, usize)>,
) -> (usize, usize, Vec<[u8; 4]>) {
    render_volume_with_step(volume, view, size, 1.0)
}

/// Orthographic CPU raycaster with a relative sample-step multiplier (`1.0` = default).
pub fn render_volume_with_step(
    volume: &VolumeObject,
    view: [f64; 2],
    size: Option<(usize, usize)>,
    step_scale: f64,
) -> (usize, usize, Vec<[u8; 4]>) {
    render_volume_impl(volume, view, size, step_scale, 1.0, None).expect("no cancellation token")
}

/// Render a display raster, returning no partial image if cancelled.
pub fn render_volume_cancellable(
    volume: &VolumeObject,
    view: [f64; 2],
    size: (usize, usize),
    cancel: &AtomicBool,
) -> Option<(usize, usize, Vec<[u8; 4]>)> {
    render_volume_impl(volume, view, Some(size), 1.0, 1.0, Some(cancel)).ok()
}

/// Render the same projection into an interactive viewport. A zoom above 1
/// narrows the viewport around the centre of the volume.
pub(crate) fn render_volume_viewport(
    volume: &VolumeObject,
    view: [f64; 2],
    size: (usize, usize),
    zoom: f64,
    cancel: &AtomicBool,
) -> Result<Vec<[u8; 4]>> {
    let (_, _, pixels) = render_volume_impl(volume, view, Some(size), 1.0, zoom, Some(cancel))?;
    Ok(pixels)
}

fn render_volume_impl(
    volume: &VolumeObject,
    view: [f64; 2],
    size: Option<(usize, usize)>,
    step_scale: f64,
    zoom: f64,
    cancel: Option<&AtomicBool>,
) -> Result<(usize, usize, Vec<[u8; 4]>)> {
    let check_cancel = || cancel.map_or(Ok(()), crate::dsp::check_cancel);
    check_cancel()?;
    let [ny, nx, nz] = volume.dims;
    let [sx, sy, sz] = volume.transformation;
    if ny == 0 || nx == 0 || nz == 0 {
        return Ok((0, 0, Vec::new()));
    }
    let wx = nx as f64 * sx;
    let wy = ny as f64 * sy;
    let wz = nz as f64 * sz;
    let half = [0.5 * wx, 0.5 * wy, 0.5 * wz];

    let m = view3d::viewmtx(view[0], view[1]);
    let right = [m[0][0], m[0][1], m[0][2]];
    let up = [m[1][0], m[1][1], m[1][2]];
    // Row 2 of viewmtx points toward the viewer; rays march into the volume.
    let ray_dir = [-m[2][0], -m[2][1], -m[2][2]];

    let u_half = 0.5 * (right[0].abs() * wx + right[1].abs() * wy + right[2].abs() * wz) / zoom;
    let v_half = 0.5 * (up[0].abs() * wx + up[1].abs() * wy + up[2].abs() * wz) / zoom;

    let (width, height) = match size {
        Some((w, h)) => (w.max(1), h.max(1)),
        None => (
            (right[0].abs() * nx as f64 + right[1].abs() * ny as f64 + right[2].abs() * nz as f64)
                .round()
                .max(1.0) as usize,
            (up[0].abs() * nx as f64 + up[1].abs() * ny as f64 + up[2].abs() * nz as f64)
                .round()
                .max(1.0) as usize,
        ),
    };

    let du = (2.0 * u_half) / width as f64;
    let dv = (2.0 * v_half) / height as f64;

    let step_scale = step_scale.max(1e-6);
    let base_step = step_scale
        / ((ray_dir[0].abs() / sx) + (ray_dir[1].abs() / sy) + (ray_dir[2].abs() / sz)).max(1e-12);
    let max_samples = ((8192.0 / step_scale.min(1.0)).ceil() as usize).min(65536);

    let style = volume.rendering_style;

    let mut pixels = vec![[0u8; 4]; width * height];
    let draw_row = |(row, row_buf): (usize, &mut [[u8; 4]])| -> Result<()> {
        #[cfg(test)]
        notify_test_ray_row(&volume.identity, row, height);
        let v = v_half - (row as f64 + 0.5) * dv;
        for (col, px) in row_buf.iter_mut().enumerate() {
            check_cancel()?;
            let u = -u_half + (col as f64 + 0.5) * du;
            let origin = [
                u * right[0] + v * up[0],
                u * right[1] + v * up[1],
                u * right[2] + v * up[2],
            ];
            let Some((t_near, t_far)) = intersect_box(origin, ray_dir, half) else {
                continue;
            };
            // Cover the entire ray within the sample budget for extreme scales.
            let step = base_step.max((t_far - t_near) / max_samples as f64);
            let ray = RayContext {
                volume,
                origin,
                ray_dir,
                half,
                t_near,
                t_far,
                step,
                max_samples,
            };
            *px = match (&volume.overlay_data, style) {
                (None, RenderingStyle::MaximumIntensityProjection) => {
                    render_mip_minip_ray(&ray, true, cancel)
                }
                (None, RenderingStyle::MinimumIntensityProjection) => {
                    render_mip_minip_ray(&ray, false, cancel)
                }
                (None, RenderingStyle::Isosurface) => render_isosurface_ray(&ray),
                (None, RenderingStyle::SlicePlanes) => render_slice_planes_ray(&ray),
                (None, RenderingStyle::VolumeRendering) => render_volume_ray(&ray),
                (Some(overlay), RenderingStyle::MaximumIntensityProjection) => {
                    composite_overlay_ray(&ray, overlay, render_mip_minip_ray(&ray, true, cancel))
                }
                (Some(overlay), RenderingStyle::MinimumIntensityProjection) => {
                    composite_overlay_ray(&ray, overlay, render_mip_minip_ray(&ray, false, cancel))
                }
                (Some(overlay), RenderingStyle::Isosurface) => {
                    composite_overlay_ray(&ray, overlay, render_isosurface_ray(&ray))
                }
                (Some(overlay), RenderingStyle::SlicePlanes) => {
                    composite_overlay_ray(&ray, overlay, render_slice_planes_ray(&ray))
                }
                (Some(overlay), RenderingStyle::VolumeRendering) => {
                    render_volume_with_overlay_ray(&ray, overlay)
                }
            };
        }
        Ok(())
    };
    // Keep small interactive viewports serial. The regular figure renderer
    // retains its existing scheduling policy.
    if cancel.is_none()
        || (width
            .saturating_mul(height)
            .saturating_mul(nx.max(ny).max(nz))
            >= 1_000_000
            && crate::parallel::current_worker_count() > 1)
    {
        pixels
            .par_chunks_mut(width)
            .enumerate()
            .try_for_each(draw_row)?;
    } else {
        pixels
            .chunks_mut(width)
            .enumerate()
            .try_for_each(draw_row)?;
    }

    check_cancel()?;
    Ok((width, height, pixels))
}

/// Render size for SVG/desktop display: preserves projected aspect ratio up to `max_dim`.
pub fn display_render_size(
    volume: &VolumeObject,
    view: [f64; 2],
    max_dim: usize,
) -> (usize, usize) {
    let [ny, nx, nz] = volume.dims;
    let [sx, sy, sz] = volume.transformation;
    let wx = nx as f64 * sx;
    let wy = ny as f64 * sy;
    let wz = nz as f64 * sz;
    let m = view3d::viewmtx(view[0], view[1]);
    let right = [m[0][0], m[0][1], m[0][2]];
    let up = [m[1][0], m[1][1], m[1][2]];
    let w = (right[0].abs() * wx + right[1].abs() * wy + right[2].abs() * wz).max(1.0);
    let h = (up[0].abs() * wx + up[1].abs() * wy + up[2].abs() * wz).max(1.0);
    let scale = if w.max(h) > max_dim as f64 {
        max_dim as f64 / w.max(h)
    } else if w.max(h) < 128.0 {
        (128.0 / w.max(h)).min(4.0)
    } else {
        1.0
    };
    (
        (w * scale).round().max(1.0) as usize,
        (h * scale).round().max(1.0) as usize,
    )
}

/// Projected 2-D bounding box `[x0, x1], [y0, y1]` of a volume in the display axes.
pub fn projected_display_bounds(axes: &super::Axes, volume: &VolumeObject) -> ([f64; 2], [f64; 2]) {
    if let Some(bounds) = volume.projected_bounds {
        return bounds;
    }
    if !view3d::is_3d(axes.view) {
        let (ex, ey, _) = volume.extent();
        return (ex, ey);
    }
    let (xr, yr) = axes.bounds();
    let zr = axes.zbounds();
    let (vx, vy, vz) = volume.extent();
    let norm = |v: f64, r: [f64; 2]| {
        if r[1] > r[0] {
            (v - r[0]) / (r[1] - r[0])
        } else {
            0.5
        }
    };
    let ux = [norm(vx[0], xr), norm(vx[1], xr)];
    let uy = [norm(vy[0], yr), norm(vy[1], yr)];
    let uz = [norm(vz[0], zr), norm(vz[1], zr)];
    let m = view3d::viewmtx(axes.view[0], axes.view[1]);
    let mut u_min = f64::INFINITY;
    let mut u_max = f64::NEG_INFINITY;
    let mut v_min = f64::INFINITY;
    let mut v_max = f64::NEG_INFINITY;
    for &x in &ux {
        for &y in &uy {
            for &z in &uz {
                let p = view3d::project(&m, [x, y, z]);
                u_min = u_min.min(p[0]);
                u_max = u_max.max(p[0]);
                v_min = v_min.min(p[1]);
                v_max = v_max.max(p[1]);
            }
        }
    }
    ([u_min, u_max], [v_min, v_max])
}

impl super::Axes {
    /// Visible volume indices in paint order, paired with the number of series
    /// slots that precede them. Slot zero is the volume background.
    pub fn volume_paint_order(&self) -> Vec<(usize, usize)> {
        use super::{AxesDataChild, PlotKind, axes_data_children};

        let volumes = &self.extras.volumes;
        let mut order: Vec<_> = (0..volumes.len()).map(|i| (0, i)).collect();
        let top_mip = |v: &VolumeObject| {
            v.visible
                && v.rendering_style == RenderingStyle::MaximumIntensityProjection
                && v.display_view.unwrap_or(self.view) == view3d::VIEW_2D
        };
        let line = |i: usize| self.series[i].visible && self.series[i].kind == PlotKind::Line;
        if volumes.iter().any(top_mip) && (0..self.series.len()).any(line) {
            // Walk back to front in public Children order. Insert the MIPs
            // between their lines while keeping the existing series paint slots.
            let mut after = 0;
            for child in axes_data_children(self).into_iter().rev() {
                match child {
                    AxesDataChild::Series(i) if line(i) => after = after.max(i + 1),
                    AxesDataChild::Volume(i) if top_mip(&volumes[i]) => order[i].0 = after,
                    _ => {}
                }
            }
            order.sort_by_key(|&(slot, _)| slot);
        }
        order.retain(|&(_, i)| volumes[i].visible && !volumes[i].dims.contains(&0));
        order
    }
}

/// Draw one volume as an embedded RGBA PNG `<image>` element in SVG.
pub(super) fn draw_svg(
    svg: &mut impl std::fmt::Write,
    axes: &super::Axes,
    volume: &VolumeObject,
    px: &dyn Fn(f64) -> f64,
    py: &dyn Fn(f64) -> f64,
    cancel: Option<&AtomicBool>,
) -> Result<()> {
    if let Some(cancel) = cancel {
        crate::dsp::check_cancel(cancel)?;
    }
    let (ex, ey) = projected_display_bounds(axes, volume);
    let [x0, x1] = ex.map(px);
    let [y0, y1] = ey.map(py);
    if ![x0, x1, y0, y1].iter().all(|v| v.is_finite()) {
        return Ok(());
    }
    let view = volume.display_view.unwrap_or(axes.view);
    let (rw, rh) = display_render_size(volume, view, 320);
    let (w, h, rgba) = render_volume_impl(volume, view, Some((rw, rh)), 1.0, 1.0, cancel)?;
    if w == 0 || h == 0 {
        return Ok(());
    }
    if let Some(cancel) = cancel {
        crate::dsp::check_cancel(cancel)?;
    }
    let (y, transform) = if volume.display_flip_y(axes) {
        (
            0.,
            format!(" transform=\"translate(0 {}) scale(1 -1)\"", y0.max(y1)),
        )
    } else {
        (y0.min(y1), String::new())
    };
    let png = super::image::encode_png_rgba(w, h, &rgba);
    if let Some(cancel) = cancel {
        crate::dsp::check_cancel(cancel)?;
    }
    write!(
        svg,
        "<image x=\"{}\" y=\"{y}\" width=\"{}\" height=\"{}\"{transform} preserveAspectRatio=\"none\" href=\"data:image/png;base64,{}\"/>",
        x0.min(x1),
        (x1 - x0).abs(),
        (y1 - y0).abs(),
        super::image::base64(&png)
    )
    .map_err(|_| "SVG write failed".into())
}

#[cfg(test)]
type RayRowHook = Box<dyn FnMut(&ObjectIdentity, usize, usize)>;

#[cfg(test)]
std::thread_local! {
    static TEST_RAY_ROW_HOOK: std::cell::RefCell<Option<RayRowHook>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn notify_test_ray_row(identity: &ObjectIdentity, row: usize, height: usize) {
    TEST_RAY_ROW_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().as_mut() {
            hook(identity, row, height);
        }
    });
}

#[cfg(test)]
struct TestRayRowGuard;

#[cfg(test)]
impl Drop for TestRayRowGuard {
    fn drop(&mut self) {
        TEST_RAY_ROW_HOOK.with(|slot| {
            *slot.borrow_mut() = None;
        });
    }
}

#[cfg(test)]
fn install_test_ray_row_hook(
    hook: impl FnMut(&ObjectIdentity, usize, usize) + 'static,
) -> TestRayRowGuard {
    TEST_RAY_ROW_HOOK.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(hook));
    });
    TestRayRowGuard
}

#[cfg(all(test, feature = "plot-export"))]
mod tests {
    use super::*;
    use crate::export::{GraphicsOptions, export_graphics_with_cancel};
    use crate::plot::Figure;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::time::Duration;

    fn small_mip_volume(val: f64) -> VolumeObject {
        let mut vol =
            VolumeObject::new([4, 4, 4], Arc::from(vec![val; 64]), ImageDataClass::Double);
        vol.rendering_style = RenderingStyle::MaximumIntensityProjection;
        vol.data_limits = [0.0, 1.0];
        vol.data_limits_manual = true;
        vol
    }

    fn temp_export_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("matrust_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn volume_export_cancel_mid_first_volume_exits_early_and_skips_second() {
        let dir = temp_export_dir("volume_export_cancel_mid");
        let existing = dir.join("existing.svg");
        let sentinel = b"KEEP_EXISTING_BYTES";
        std::fs::write(&existing, sentinel).unwrap();

        let v1 = small_mip_volume(1.0);
        let v2 = small_mip_volume(0.5);
        let v1_id = v1.identity.clone();
        let v2_id = v2.identity.clone();

        let mut figure = Figure::default();
        figure.axes.extras.volumes.push(v1);
        figure.axes.extras.volumes.push(v2);

        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_worker = Arc::clone(&cancel);
        let (entered_tx, entered_rx) = mpsc::sync_channel::<()>(1);
        let (stopped_tx, stopped_rx) = mpsc::sync_channel::<()>(1);

        let stopper = std::thread::spawn(move || {
            entered_rx
                .recv_timeout(Duration::from_secs(5))
                .expect("timed out waiting for first volume raycast to start");
            cancel_worker.store(true, Ordering::Relaxed);
            stopped_tx
                .send(())
                .expect("failed to notify export thread after setting cancel");
        });

        let v1_rows = Arc::new(AtomicUsize::new(0));
        let v1_height = Arc::new(AtomicUsize::new(0));
        let v2_started = Arc::new(AtomicBool::new(false));

        let v1_rows_hook = Arc::clone(&v1_rows);
        let v1_height_hook = Arc::clone(&v1_height);
        let v2_started_hook = Arc::clone(&v2_started);

        let _guard = install_test_ray_row_hook(move |id, row, height| {
            if id.same_object(&v1_id) {
                v1_rows_hook.fetch_add(1, Ordering::Relaxed);
                v1_height_hook.store(height, Ordering::Relaxed);
                if row == 1 {
                    entered_tx
                        .send(())
                        .expect("failed to signal stop thread from first volume");
                    stopped_rx
                        .recv_timeout(Duration::from_secs(5))
                        .expect("timed out waiting for stop signal");
                }
            } else if id.same_object(&v2_id) {
                v2_started_hook.store(true, Ordering::Relaxed);
            }
        });

        let result = export_graphics_with_cancel(
            &figure,
            &existing,
            "svg",
            GraphicsOptions::default(),
            &cancel,
        );
        stopper.join().expect("stopper thread panicked");

        assert!(result.is_err(), "expected cancelled export to fail");
        let rows_done = v1_rows.load(Ordering::Relaxed);
        let total_height = v1_height.load(Ordering::Relaxed);
        assert!(
            rows_done >= 2 && rows_done < total_height,
            "expected first volume to exit early after row 1 (rows_done={rows_done}, total_height={total_height})"
        );
        assert!(
            !v2_started.load(Ordering::Relaxed),
            "second volume must never start after cancellation"
        );
        assert_eq!(
            std::fs::read(&existing).unwrap(),
            sentinel,
            "existing destination must remain unchanged on cancel"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn volume_export_cancel_preset_token_skips_all_volumes_and_leaves_no_new_file() {
        let dir = temp_export_dir("volume_export_cancel_preset");
        let new_file = dir.join("should_not_exist.svg");

        let mut figure = Figure::default();
        figure.axes.extras.volumes.push(small_mip_volume(1.0));
        figure.axes.extras.volumes.push(small_mip_volume(0.5));

        let any_started = Arc::new(AtomicBool::new(false));
        let any_started_hook = Arc::clone(&any_started);
        let _guard = install_test_ray_row_hook(move |_, _, _| {
            any_started_hook.store(true, Ordering::Relaxed);
        });

        let cancel = AtomicBool::new(true);
        let result = export_graphics_with_cancel(
            &figure,
            &new_file,
            "svg",
            GraphicsOptions::default(),
            &cancel,
        );

        assert!(result.is_err(), "expected preset-cancelled export to fail");
        assert!(
            !any_started.load(Ordering::Relaxed),
            "no volume raycast should start when token is already set"
        );
        assert!(
            !new_file.exists(),
            "no destination file should be created on cancel"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn volume_export_cancel_uncancelled_small_volume_exports_expected_rgba() {
        let dir = temp_export_dir("volume_export_cancel_ok");
        let out_png = dir.join("small_vol.png");

        let mut vol = small_mip_volume(1.0);
        vol.colormap = Arc::new(vec![[1.0, 0.0, 0.0], [1.0, 0.0, 0.0]]);
        let (rw, rh) = display_render_size(&vol, view3d::VIEW_2D, 320);
        let (ew, eh, expected_rgba) = render_volume(&vol, view3d::VIEW_2D, Some((rw, rh)));
        let expected_b64 = crate::plot::image::base64(&crate::plot::image::encode_png_rgba(
            ew,
            eh,
            &expected_rgba,
        ));

        let mut figure = Figure::default();
        figure.axes.extras.volumes.push(vol);

        let cancel = AtomicBool::new(false);
        let svg = figure.svg_without_background(&cancel).unwrap();
        let prefix = "href=\"data:image/png;base64,";
        let start = svg.find(prefix).expect("expected embedded PNG in SVG") + prefix.len();
        let end = start + svg[start..].find('"').unwrap();
        assert_eq!(&svg[start..end], expected_b64);

        export_graphics_with_cancel(
            &figure,
            &out_png,
            "png",
            GraphicsOptions::default(),
            &cancel,
        )
        .expect("uncancelled small volume export should succeed");

        let bytes = std::fs::read(&out_png).unwrap();
        assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
        let img = image::load_from_memory(&bytes).unwrap().to_rgba8();
        let (w, h) = img.dimensions();
        assert!(w > 10 && h > 10);
        assert_eq!(img.get_pixel(w / 2, h / 2).0, [255, 0, 0, 255]);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
