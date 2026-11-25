//! Image objects (`image`, `imagesc`, `imshow`) on the desktop: one
//! nearest-neighbour texture per image, cached in egui memory and rebuilt only
//! when its data, colormap, CLim or orientation change. The `-batch` PNG draws
//! the same raster through `matrust::plot::screen_raster`.
use eframe::egui;
use matrust::plot::{Axes, screen_raster};
use std::hash::{Hash, Hasher};

/// Adds the axes' ordinary images to the plot background.
pub(super) fn add(ui: &mut egui_plot::PlotUi<'_>, axes: &Axes, plot_id: egui::Id) {
    let extras = &axes.extras;
    if extras.images.is_empty() {
        return;
    }
    let map = extras.effective_colormap();
    let clim = extras.color_limits();
    for (k, image) in extras.images.iter().enumerate() {
        if !image.visible || image.rows == 0 || image.cols == 0 {
            continue;
        }
        let (ex, ey) = image.edges();
        let row1_top = extras.y_reversed == (ey[0] < ey[1]);
        let (flip_rows, flip_cols) = (!row1_top, ex[0] > ex[1]);
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (std::sync::Arc::as_ptr(&image.data) as *const f64 as usize).hash(&mut hasher);
        (image.rows, image.cols, flip_rows, flip_cols).hash(&mut hasher);
        format!("{:?}", image.mapping).hash(&mut hasher);
        clim.map(f64::to_bits).hash(&mut hasher);
        for row in map.iter() {
            row.map(f64::to_bits).hash(&mut hasher);
        }
        let key = hasher.finish();
        let id = plot_id.with(("matrust_image", k));
        let ctx = ui.ctx().clone();
        let cached = ctx.data(|d| d.get_temp::<(u64, egui::TextureHandle)>(id));
        let texture = match cached {
            Some((cached_key, texture)) if cached_key == key => texture,
            _ => {
                let rgb = screen_raster(image, &map, clim, flip_rows, flip_cols);
                let texture = ctx.load_texture(
                    format!("matrust-image-{k}"),
                    egui::ColorImage::from_rgb([image.cols, image.rows], &rgb),
                    egui::TextureOptions::NEAREST,
                );
                ctx.data_mut(|d| d.insert_temp(id, (key, texture.clone())));
                texture
            }
        };
        let center = egui_plot::PlotPoint::new((ex[0] + ex[1]) / 2., (ey[0] + ey[1]) / 2.);
        let size = egui::vec2((ex[1] - ex[0]).abs() as f32, (ey[1] - ey[0]).abs() as f32);
        ui.image(egui_plot::PlotImage::new("", texture.id(), center, size).allow_hover(false));
    }
}
