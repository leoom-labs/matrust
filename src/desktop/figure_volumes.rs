//! Background volume rasterization for desktop figure panes.
use eframe::egui;
use egui_plot::{
    PlotBounds, PlotGeometry, PlotImage, PlotItem, PlotItemBase, PlotPoint, PlotTransform,
};
use matrust::ObjectIdentity;
use matrust::plot::{
    Axes, VolumeObject, display_render_size, projected_display_bounds, render_volume_cancellable,
};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

const MAX_RASTER_SIDE: usize = 320;
type VolumeTextures = Arc<Mutex<Vec<VolumeTexture>>>;

struct RasterKey {
    data_revision: ObjectIdentity,
    inputs: u64,
}

impl RasterKey {
    fn new(
        volume: &VolumeObject,
        view: [f64; 2],
        size: (usize, usize),
        overlay_revision: u64,
    ) -> Self {
        let mut hash = DefaultHasher::new();
        (volume.data.as_ptr() as usize, volume.dims, size).hash(&mut hash);
        view.map(f64::to_bits).hash(&mut hash);
        volume.rendering_style.hash(&mut hash);
        volume.transformation.map(f64::to_bits).hash(&mut hash);
        volume.data_limits.map(f64::to_bits).hash(&mut hash);
        volume.isosurface_value.to_bits().hash(&mut hash);
        volume.isosurface_color.map(f64::to_bits).hash(&mut hash);
        volume.slice_plane_values.len().hash(&mut hash);
        for plane in &volume.slice_plane_values {
            plane.map(f64::to_bits).hash(&mut hash);
        }
        volume.colormap.len().hash(&mut hash);
        for row in volume.colormap.iter() {
            row.map(f64::to_bits).hash(&mut hash);
        }
        volume.alphamap.len().hash(&mut hash);
        for &alpha in volume.alphamap.iter() {
            alpha.to_bits().hash(&mut hash);
        }
        if volume.overlay_data.is_some() {
            overlay_revision.hash(&mut hash);
            volume.overlay_rendering_style.hash(&mut hash);
            volume
                .overlay_display_range
                .map(f64::to_bits)
                .hash(&mut hash);
        }
        Self {
            data_revision: volume.data_revision().clone(),
            inputs: hash.finish(),
        }
    }

    fn matches(&self, other: &Self) -> bool {
        self.inputs == other.inputs && self.data_revision.same_object(&other.data_revision)
    }
}

struct Pending {
    key: RasterKey,
    cancel: Arc<AtomicBool>,
    result: mpsc::Receiver<Option<egui::ColorImage>>,
}

impl Drop for Pending {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

struct VolumeTexture {
    identity: ObjectIdentity,
    overlay_buffers: Option<OverlayBuffers>,
    overlay_revision: u64,
    finished: Option<(RasterKey, egui::TextureHandle)>,
    pending: Option<Pending>,
}

struct OverlayBuffers {
    data: Arc<[f64]>,
    colormap: Arc<Vec<[f64; 3]>>,
    alphamap: Arc<Vec<f64>>,
}

impl VolumeTexture {
    // Keep the current allocations alive for pointer comparison. A held
    // texture needs only its revision when the overlay buffers are replaced.
    fn update_overlay_revision(&mut self, volume: &VolumeObject) -> u64 {
        match (&self.overlay_buffers, &volume.overlay_data) {
            (None, None) => {}
            (Some(last), Some(mask))
                if Arc::ptr_eq(&last.data, &mask.data)
                    && Arc::ptr_eq(&last.colormap, &volume.overlay_colormap)
                    && Arc::ptr_eq(&last.alphamap, &volume.overlay_alphamap) => {}
            (_, mask) => {
                self.overlay_buffers = mask.as_ref().map(|mask| OverlayBuffers {
                    data: mask.data.clone(),
                    colormap: volume.overlay_colormap.clone(),
                    alphamap: volume.overlay_alphamap.clone(),
                });
                self.overlay_revision += 1;
            }
        }
        self.overlay_revision
    }

    fn update(
        &mut self,
        ctx: &egui::Context,
        volume: &VolumeObject,
        view: [f64; 2],
        size: (usize, usize),
    ) {
        let key = RasterKey::new(volume, view, size, self.update_overlay_revision(volume));
        if let Some(pending) = &self.pending {
            if !pending.key.matches(&key) {
                pending.cancel.store(true, Ordering::Relaxed);
            }
            match pending.result.try_recv() {
                Err(mpsc::TryRecvError::Empty) => return,
                Ok(Some(image)) if !pending.cancel.load(Ordering::Relaxed) => {
                    let texture =
                        ctx.load_texture("matrust-volume", image, egui::TextureOptions::LINEAR);
                    self.finished = Some((key, texture));
                    self.pending = None;
                    return;
                }
                _ => self.pending = None,
            }
        }
        if self
            .finished
            .as_ref()
            .is_some_and(|(last, _)| last.matches(&key))
        {
            return;
        }

        let (send, result) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        self.pending = Some(Pending {
            key,
            cancel: cancel.clone(),
            result,
        });
        let viewport = ctx.viewport_id();
        let ctx = ctx.clone();
        let volume = volume.clone();
        #[cfg(test)]
        let before_render = BEFORE_RENDER.with(|hook| hook.borrow_mut().take());
        // Let the cancelled task finish before submitting a replacement. Rapid
        // camera changes then coalesce into the latest request at the next paint.
        rayon::spawn(move || {
            #[cfg(test)]
            if let Some(hook) = before_render {
                hook(&cancel);
            }
            let image =
                render_volume_cancellable(&volume, view, size, &cancel).map(|(w, h, rgba)| {
                    egui::ColorImage::from_rgba_unmultiplied([w, h], bytemuck::cast_slice(&rgba))
                });
            if send.send(image).is_ok() {
                ctx.request_repaint_of(viewport);
            }
        });
    }
}

/// PlotImage paints directly, ahead of the plot's queued shape list. A volume
/// above a line needs its quad in the same shape list as that line.
pub(super) struct VolumeQuad {
    base: PlotItemBase,
    bounds: PlotBounds,
    id: egui::TextureId,
    tint: egui::Color32,
    uv: egui::Rect,
}

impl PlotItem for VolumeQuad {
    fn shapes(&self, _ui: &egui::Ui, transform: &PlotTransform, shapes: &mut Vec<egui::Shape>) {
        if self.tint == egui::Color32::TRANSPARENT {
            return;
        }
        let a = transform.position_from_point(&self.bounds.min().into());
        let b = transform.position_from_point(&self.bounds.max().into());
        if ![a.x, a.y, b.x, b.y].iter().all(|v| v.is_finite()) {
            return;
        }
        let mut mesh = egui::Mesh::with_texture(self.id);
        mesh.add_rect_with_uv(egui::Rect::from_two_pos(a, b), self.uv, self.tint);
        shapes.push(egui::Shape::mesh(mesh));
    }
    fn initialize(&mut self, _range: std::ops::RangeInclusive<f64>) {}
    fn color(&self) -> egui::Color32 {
        egui::Color32::TRANSPARENT
    }
    fn allow_hover(&self) -> bool {
        false
    }
    fn geometry(&self) -> PlotGeometry<'_> {
        PlotGeometry::None
    }
    fn bounds(&self) -> PlotBounds {
        self.bounds
    }
    fn base(&self) -> &PlotItemBase {
        &self.base
    }
    fn base_mut(&mut self) -> &mut PlotItemBase {
        &mut self.base
    }
}

pub(super) fn add_background(
    ui: &mut egui_plot::PlotUi<'_>,
    axes: &Axes,
    plot_id: egui::Id,
) -> Vec<(usize, VolumeQuad)> {
    let ctx = ui.ctx().clone();
    let cache_id = plot_id.with("matrust_volumes");
    let order = axes.volume_paint_order();
    if order.is_empty() {
        forget(&ctx, plot_id);
        return Vec::new();
    }
    let cache = ctx.data_mut(|data| {
        data.get_temp_mut_or_default::<VolumeTextures>(cache_id)
            .clone()
    });
    let mut textures = cache.lock().unwrap();
    textures.retain(|texture| {
        order.iter().any(|&(_, i)| {
            axes.extras.volumes[i]
                .identity
                .same_object(&texture.identity)
        })
    });
    let mut deferred = Vec::new();
    for (after, i) in order {
        let volume = &axes.extras.volumes[i];
        let index = if let Some(index) = textures
            .iter()
            .position(|t| t.identity.same_object(&volume.identity))
        {
            index
        } else {
            textures.push(VolumeTexture {
                identity: volume.identity.clone(),
                overlay_buffers: None,
                overlay_revision: 0,
                finished: None,
                pending: None,
            });
            textures.len() - 1
        };
        let texture = &mut textures[index];
        let view = volume.display_view.unwrap_or(axes.view);
        texture.update(
            &ctx,
            volume,
            view,
            display_render_size(volume, view, MAX_RASTER_SIDE),
        );
        let (id, tint) = texture.finished.as_ref().map_or(
            (egui::TextureId::default(), egui::Color32::TRANSPARENT),
            |(_, image)| (image.id(), egui::Color32::WHITE),
        );
        let (ex, ey) = projected_display_bounds(axes, volume);
        let (top, bottom) = if volume.display_flip_y(axes) {
            (1., 0.)
        } else {
            (0., 1.)
        };
        let uv = egui::Rect::from_min_max(egui::pos2(0., top), egui::pos2(1., bottom));
        if after == 0 {
            let center = PlotPoint::new((ex[0] + ex[1]) / 2., (ey[0] + ey[1]) / 2.);
            let size = egui::vec2((ex[1] - ex[0]).abs() as f32, (ey[1] - ey[0]).abs() as f32);
            ui.image(
                PlotImage::new("", id, center, size)
                    .uv(uv)
                    .tint(tint)
                    .allow_hover(false),
            );
        } else {
            deferred.push((
                after,
                VolumeQuad {
                    base: PlotItemBase::new(String::new()),
                    bounds: PlotBounds::from_min_max([ex[0], ey[0]], [ex[1], ey[1]]),
                    id,
                    tint,
                    uv,
                },
            ));
        }
    }
    deferred
}

pub(super) fn forget(ctx: &egui::Context, plot_id: egui::Id) {
    ctx.data_mut(|data| data.remove::<VolumeTextures>(plot_id.with("matrust_volumes")));
}

#[cfg(test)]
type RenderHook = Box<dyn FnOnce(&Arc<AtomicBool>) + Send>;

#[cfg(test)]
thread_local! {
    pub(super) static BEFORE_RENDER: std::cell::RefCell<Option<RenderHook>> = const { std::cell::RefCell::new(None) };
}
