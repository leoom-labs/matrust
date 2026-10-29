use super::*;
use crate::theme::ThemedScrollArea;

struct PreviewSaveIntent {
    viewport: egui::ViewportId,
    frame: u64,
    interaction: u64,
    source: PathBuf,
    pixels: Arc<egui::ColorImage>,
    format: image_export::Format,
}

pub struct Viewer {
    save_intent: Option<PreviewSaveIntent>,
    save_finished: Option<(egui::ViewportId, u64)>,
    pub navigation: matrust::session::PreviewNavigation,
    pub restore_window: bool,
    pub path: PathBuf,
    pub original_size: [u32; 2],
    pub preview_size: [usize; 2],
    pub fit: bool,
    pub zoom: f32,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub(super) pixels: Arc<egui::ColorImage>,
    pub(super) original: Option<image_load::Original>,
    pub(super) export_format: image_export::Format,
    pub page: Option<image_pages::Page>,
    pub page_number: u32,
    format: &'static str,
    file_bytes: usize,
    texture: egui::TextureHandle,
}
impl Viewer {
    fn new(
        ctx: &egui::Context,
        image: image_load::Preview,
        page: Option<image_pages::Page>,
    ) -> Self {
        let preview_size = image.pixels.size;
        let pixels = Arc::new(image.pixels);
        let texture = ctx.load_texture(
            "plot_image_preview",
            egui::ImageData::Color(pixels.clone()),
            egui::TextureOptions::LINEAR,
        );
        Self {
            save_intent: None,
            save_finished: None,
            navigation: Default::default(),
            restore_window: false,
            path: image.path,
            original: image.original,
            original_size: image.original_size,
            preview_size,
            fit: true,
            zoom: 1.,
            error: None,
            notice: None,
            pixels,
            export_format: image_export::Format::default(),
            page_number: page.as_ref().map_or(1, |p| p.index + 1),
            page,
            format: image.format,
            file_bytes: image.file_bytes,
            texture,
        }
    }
}
fn show_pixels(ui: &mut egui::Ui, viewer: &mut Viewer) {
    let natural = egui::vec2(viewer.preview_size[0] as f32, viewer.preview_size[1] as f32);
    // Read the finite visible viewport before entering a potentially unbounded
    // scroll content Ui. Manual zoom retains its original scrolling layout.
    let visible = ui.available_rect_before_wrap().intersect(ui.clip_rect());
    let available = if viewer.fit {
        visible.size()
    } else {
        ui.available_size()
    }
    .max(egui::vec2(1., 1.));
    let scale = if viewer.fit {
        (available.x / natural.x)
            .min(available.y / natural.y)
            .min(1.)
    } else {
        viewer.zoom
    };
    if viewer.fit {
        viewer.zoom = scale;
    }
    if viewer.fit {
        viewer.navigation.scroll = [0.; 2];
        let _ = ui.allocate_exact_size(available, egui::Sense::hover());
        let image_rect = egui::Rect::from_center_size(visible.center(), natural * scale);
        ui.place(
            image_rect,
            egui::Image::new((viewer.texture.id(), natural * scale))
                .fit_to_exact_size(natural * scale),
        );
        return;
    }
    let scroll = egui::ScrollArea::both()
        .id_salt((
            "plot_image_scroll",
            &viewer.path,
            viewer.page.as_ref().map(|p| p.index),
        ))
        .scroll_offset(egui::vec2(
            viewer.navigation.scroll[0],
            viewer.navigation.scroll[1],
        ))
        .auto_shrink([false, false])
        .show_themed(ui, |ui| {
            ui.add(
                egui::Image::new((viewer.texture.id(), natural * scale))
                    .fit_to_exact_size(natural * scale),
            );
        });
    viewer.navigation.scroll = [scroll.state.offset.x, scroll.state.offset.y];
}

impl Workspace {
    fn preview_save_allowed(&self, ctx: &egui::Context) -> bool {
        !self.busy
            && !self.file_action_pending()
            && !self.allow_close
            && !self.pending_quit
            && !self.recovering_session
            && self.pending_image.is_none()
            && ctx.memory(|memory| memory.top_modal_layer().is_none())
            && !egui::Popup::is_any_open(ctx)
            && ctx.input(|input| {
                input.viewport().focused != Some(false) && !input.viewport().close_requested()
            })
    }
    fn queue_preview_save(&mut self, ctx: &egui::Context, clicked: bool) {
        if !clicked || !self.preview_save_allowed(ctx) {
            return;
        }
        let stamp = (ctx.viewport_id(), ctx.cumulative_frame_nr());
        let interaction = self.document_save.interaction_generation();
        if let Some(viewer) = &mut self.image_view
            && viewer.save_finished != Some(stamp)
            && viewer.save_intent.is_none()
        {
            viewer.save_intent = Some(PreviewSaveIntent {
                viewport: stamp.0,
                frame: stamp.1,
                interaction,
                source: viewer.path.clone(),
                pixels: viewer.pixels.clone(),
                format: viewer.export_format,
            });
        }
    }
    pub(super) fn cancel_preview_save(&mut self) {
        if let Some(viewer) = &mut self.image_view
            && let Some(intent) = viewer.save_intent.take()
        {
            viewer.save_finished = Some((intent.viewport, intent.frame));
        }
    }
    pub(super) fn finish_preview_save(&mut self, ctx: &egui::Context) {
        if ctx.will_discard() {
            return;
        }
        let Some(viewer) = &mut self.image_view else {
            return;
        };
        if viewer
            .save_intent
            .as_ref()
            .is_some_and(|intent| intent.viewport != ctx.viewport_id())
        {
            return;
        }
        let Some(intent) = viewer.save_intent.take() else {
            return;
        };
        let stamp = (intent.viewport, intent.frame);
        if viewer.save_finished == Some(stamp) {
            return;
        }
        viewer.save_finished = Some(stamp);
        if intent.frame != ctx.cumulative_frame_nr()
            || viewer.path != intent.source
            || !Arc::ptr_eq(&viewer.pixels, &intent.pixels)
            || viewer.export_format != intent.format
            || intent.interaction != self.document_save.interaction_generation()
            || !self.preview_save_allowed(ctx)
        {
            return;
        }
        self.save_preview_copy();
    }
    pub(super) fn save_preview_copy(&mut self) {
        if self.busy {
            return;
        }
        // Generic Save and the preview button share one picker admission.
        self.cancel_preview_save();
        let Some(viewer) = &self.image_view else {
            return;
        };
        let stem = viewer
            .path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy();
        let page = viewer
            .page
            .as_ref()
            .map_or(String::new(), |p| format!("_page{}", p.index + 1));
        if let Some(path) = pick_preview_copy(&viewer.path, &stem, &page, viewer.export_format) {
            self.export_preview_to(path);
        }
    }
    pub(super) fn export_preview_to(&mut self, path: PathBuf) {
        if self.busy {
            return;
        }
        let Some(viewer) = &mut self.image_view else {
            return;
        };
        match viewer.export_format.path(path) {
            Ok(path) => {
                let job = Job::ExportPreview(image_export::Request {
                    path,
                    source: viewer.path.clone(),
                    pixels: viewer.pixels.clone(),
                });
                self.submit(job);
            }
            Err(error) => viewer.error = Some(error),
        }
    }
    pub(super) fn close_image_preview(&mut self) {
        self.cancel_original_copy();
        self.close_preview_export();
        self.document_save.preview_closed();
        self.preview_restore = None;
        self.eps_preview.pending = None;
        self.eps_preview.loading = None;
        self.recovery_dirty = true;
        self.image_view = None;
        self.pending_image = None;
        self.pending_page = None;
        if self.image_loading {
            self.image_ignore_result = true;
            self.worker.cancel.store(true, Ordering::Relaxed);
        }
    }
    pub(super) fn image_view_ui(&mut self, ctx: &egui::Context) {
        if let Some(image) = self.pending_image.take() {
            self.cancel_preview_export();
            self.clear_original_copy_picker();
            let mut viewer = Viewer::new(ctx, image, self.pending_page.take());
            if let Some(old) = &self.image_view
                && old.path == viewer.path
            {
                viewer.fit = old.fit;
                viewer.zoom = old.zoom;
                viewer.export_format = old.export_format;
                viewer.navigation.window = old.navigation.window;
                if old.page.as_ref().map(|p| p.index) == viewer.page.as_ref().map(|p| p.index) {
                    viewer.navigation.scroll = old.navigation.scroll;
                }
            }
            if let Some(restore) = self.preview_restore.take() {
                viewer.restore_view(restore.state);
            }
            self.image_view = Some(viewer);
            self.document_save.preview_installed();
            self.recovery_dirty = true;
        }
        let eps_settings_enabled = !self.busy && !self.file_action_pending();
        let Some(viewer) = &mut self.image_view else {
            return;
        };
        let before = (
            viewer.fit,
            if viewer.fit { 1. } else { viewer.zoom },
            viewer.export_format,
            viewer.navigation,
        );
        let mut open = true;
        let mut reload = false;
        let mut configure_eps = false;
        let mut close = false;
        let mut page_request = None;
        let mut save = false;
        let mut save_original = false;
        let mut copy = false;
        let viewport = ctx.content_rect().shrink(8.);
        let maximum = (viewport.size() - egui::vec2(24., 64.)).max(egui::vec2(360., 220.));
        let mut window = egui::Window::new("Plot image preview")
            .id(egui::Id::new("plot_image_preview_window"))
            .open(&mut open)
            .resizable(true)
            .default_size([820., 560.])
            .min_size([360., 220.])
            .max_size(maximum)
            .constrain_to(viewport);
        // Session sizes remain inner content sizes across egui versions.
        // Window sizing now includes chrome, so measure its actual extent.
        let chrome_id = egui::Id::new("preview_window_chrome");
        let chrome = ctx
            .data(|d| d.get_temp::<egui::Vec2>(chrome_id))
            .unwrap_or_default();
        let mut restored_content_size = None;
        if viewer.restore_window
            && let Some(saved) = viewer.navigation.window
        {
            let content_max = (maximum - chrome).max(egui::vec2(360., 220.));
            let content =
                egui::vec2(saved.size[0], saved.size[1]).clamp(egui::vec2(360., 220.), content_max);
            let size = content + chrome;
            restored_content_size = Some(content);
            let position = egui::pos2(saved.position[0], saved.position[1])
                .clamp(viewport.min, (viewport.max - size).max(viewport.min));
            // Apply once, then return control to native egui dragging/resizing.
            window = window
                .current_pos(position)
                .resize(|r| r.min_size(size).max_size(size));
            viewer.restore_window = false;
        }
        let shown = window.show(ctx, |ui| {
            let content_size = ui.max_rect().size();
            ui.label(
                viewer
                    .path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy(),
            )
            .on_hover_text(crate::path_label::display(&viewer.path).to_string());
            ui.horizontal_wrapped(|ui| {
                if ui.button("Fit image").clicked() {
                    viewer.fit = true;
                }
                if ui.button("1:1 preview").clicked() {
                    viewer.fit = false;
                    viewer.zoom = 1.;
                }
                if ui.button("Zoom out").clicked() {
                    viewer.fit = false;
                    viewer.zoom = (viewer.zoom / 1.25).max(0.01);
                }
                if ui.button("Zoom in").clicked() {
                    viewer.fit = false;
                    viewer.zoom = (viewer.zoom * 1.25).clamp(0.01, 8.);
                }
                if !viewer.fit {
                    ui.label(format!("{:.0}%", viewer.zoom * 100.));
                }
                reload = ui
                    .add_enabled(!self.busy, egui::Button::new("Reload image"))
                    .clicked();
                if eps_preview::is_eps(&viewer.path) {
                    configure_eps = ui
                        .add_enabled(eps_settings_enabled, egui::Button::new("EPS renderer..."))
                        .clicked();
                }
                close = ui.button("Close preview").clicked();
            });
            ui.add_enabled_ui(!self.busy, |ui| {
                ui.horizontal_wrapped(|ui| {
                    egui::ComboBox::from_id_salt("preview_export_format")
                        .selected_text(viewer.export_format.label())
                        .show_ui(ui, |ui| {
                            for format in image_export::Format::ALL {
                                ui.selectable_value(
                                    &mut viewer.export_format,
                                    format,
                                    format.label(),
                                );
                            }
                        });
                    save = ui.button("Save preview...").clicked();
                    save_original = ui.add_enabled(viewer.original.is_some(), egui::Button::new("Save original copy...")).on_hover_text("Copy all original bytes, pages, vectors and bit depth; no preview conversion").clicked();
                    copy = ui.button("Copy image").clicked();
                    ui.weak(format!(
                        "{} x {} | 8-bit preview",
                        viewer.preview_size[0], viewer.preview_size[1]
                    ));
                });
            });
            ui.weak("PNG/TIFF preserve transparency; JPEG/BMP and clipboard use white.");
            if let Some(error) = &viewer.error {
                ui.colored_label(tokens::palette(ui.ctx()).error, error);
            }
            if let Some(notice) = &viewer.notice {
                ui.label(notice);
            }
            if let Some(page) = &viewer.page {
                ui.add_enabled_ui(!self.busy, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .add_enabled(page.index > 0, egui::Button::new("Previous page"))
                            .clicked()
                        {
                            page_request = Some(page.index - 1);
                        }
                        ui.label("Page");
                        ui.add(
                            egui::DragValue::new(&mut viewer.page_number)
                                .range(1..=page.document.page_count),
                        );
                        ui.label(format!("of {}", page.document.page_count));
                        if ui.button("Go to page").clicked() {
                            page_request = Some(viewer.page_number - 1);
                        }
                        if ui
                            .add_enabled(
                                page.index + 1 < page.document.page_count,
                                egui::Button::new("Next page"),
                            )
                            .clicked()
                        {
                            page_request = Some(page.index + 1);
                        }
                    })
                });
                ui.weak(format!(
                    "Showing page {}. Reload image reads changes from disk.",
                    page.index + 1
                ));
            }
            ui.weak(format!(
                "{} x {} | {} | {:.1} KiB",
                viewer.original_size[0],
                viewer.original_size[1],
                viewer.format,
                viewer.file_bytes as f64 / 1024.
            ));
            if viewer.preview_size != viewer.original_size.map(|v| v as usize) {
                ui.label(format!(
                    "Display preview reduced to {} x {}. Original file is unchanged.",
                    viewer.preview_size[0], viewer.preview_size[1]
                ));
            }
            show_pixels(ui, viewer);
            content_size
        });
        if let Some(shown) = shown {
            self.document_save.register(
                document_save::Pane::Preview,
                shown.response.rect.intersect(ctx.content_rect()),
                shown.response.layer_id,
            );
            let position = [shown.response.rect.min.x, shown.response.rect.min.y];
            if let Some(size) = shown.inner {
                let measured = (shown.response.rect.size() - size).max(egui::Vec2::ZERO);
                ctx.data_mut(|d| d.insert_temp(chrome_id, measured));
                if restored_content_size.is_some_and(|target| (target - size).length() > 0.1)
                    && (measured - chrome).length() > 0.1
                {
                    // A fresh context learns the title/frame size on its first
                    // pass, then applies the saved content size on the next.
                    viewer.restore_window = true;
                    ctx.request_repaint();
                } else {
                    viewer.navigation.window = Some(matrust::session::PreviewWindow {
                        position,
                        size: [size.x, size.y],
                    });
                }
            } else if let Some(saved) = &mut viewer.navigation.window {
                saved.position = position;
            }
        }
        self.recovery_dirty |= before
            != (
                viewer.fit,
                if viewer.fit { 1. } else { viewer.zoom },
                viewer.export_format,
                viewer.navigation,
            );
        self.eps_preview.configure_from_view(
            ctx,
            &viewer.path,
            configure_eps,
            eps_settings_enabled,
        );
        if reload {
            let path = viewer.path.clone();
            if eps_preview::is_eps(&path) {
                self.begin_eps_preview(path);
                return;
            }
            let job = if let Some(page) = &viewer.page {
                Job::OpenPagedImage(path, page.index)
            } else {
                Job::OpenImage(path)
            };
            self.submit(job);
        } else if let Some(index) = page_request {
            let document = viewer.page.as_ref().unwrap().document.clone();
            self.submit(Job::ImagePage(image_pages::Page { document, index }));
        } else if copy {
            let pixels = viewer.pixels.clone();
            self.submit(Job::CopyPreview(pixels));
        }
        self.queue_preview_save(ctx, save);
        if !open || close {
            self.close_image_preview();
        } else {
            self.finish_original_copy_picker(ctx, save_original);
        }
    }
}

#[cfg(test)]
#[path = "image_view_fit_tests.rs"]
mod fit_tests;

fn pick_preview_copy(
    source: &std::path::Path,
    stem: &str,
    page: &str,
    format: image_export::Format,
) -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(choice) = PREVIEW_PICKER.with(|slot| {
        slot.borrow_mut().as_mut().map(|(choice, calls)| {
            *calls += 1;
            choice.clone()
        })
    }) {
        return choice;
    }
    rfd::FileDialog::new()
        .set_directory(source.parent().unwrap_or(std::path::Path::new(".")))
        .add_filter(format.label(), format.extensions())
        .set_file_name(format!("{stem}{page}_preview.{}", format.extensions()[0]))
        .save_file()
}
#[cfg(test)]
thread_local! {
    static PREVIEW_PICKER: std::cell::RefCell<Option<(Option<PathBuf>, usize)>> = const { std::cell::RefCell::new(None) };
}
#[cfg(test)]
#[path = "preview_save_intent_tests.rs"]
mod save_intent_tests;
