use super::{App, AppKind, Host, Outputs, Snapshot, dicom_browser, number};
use crate::{panels::PaneBorder, theme::ThemedScrollArea, tokens, toolkits::Action};
use eframe::egui;
use matrust::{
    Array, Engine, NumericClass, Result,
    engine::volume_viewer_app_hooks::{
        Launch, MAX_VIEW_PIXELS_PER_EDGE, Viewport, ZOOM_RANGE, prepare_volume, volume_shape,
    },
    plot::{
        ImageDataClass, MAX_COLORMAP_ROWS, OverlayDataClass, OverlayRenderingStyle, OverlayVolume,
        RenderingStyle, SliceDirection, VolumeObject, colormap, display_render_size,
        slice_viewer::extract_slice_image, view3d,
    },
};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex, atomic::Ordering},
};

#[path = "volume_viewer_dicom.rs"]
mod dicom;

#[cfg(test)]
#[path = "../volume_viewer_tests.rs"]
mod volume_viewer_tests;

pub(crate) type OpenSlot = Arc<Mutex<Option<Launch>>>;
const SAMPLE: &str = include_str!("volume_viewer_sample.m");
const STYLES: [(RenderingStyle, &str); 5] = [
    (RenderingStyle::VolumeRendering, "Volume rendering"),
    (
        RenderingStyle::MaximumIntensityProjection,
        "Maximum intensity (MIP)",
    ),
    (
        RenderingStyle::MinimumIntensityProjection,
        "Minimum intensity (MinIP)",
    ),
    (RenderingStyle::Isosurface, "Isosurface"),
    (RenderingStyle::SlicePlanes, "Slice planes"),
];
const PLANES: [(&str, SliceDirection, [usize; 2]); 3] = [
    ("XY", SliceDirection::Z, [0, 1]),
    ("YZ", SliceDirection::X, [2, 1]),
    ("XZ", SliceDirection::Y, [0, 2]),
];
const COLORMAPS: [&str; 5] = ["gray", "parula", "hot", "bone", "jet"];
const SPACING: std::ops::RangeInclusive<f64> = 1e-6..=1e6;
const HISTOGRAM_BINS: usize = 64;
const ALPHAMAP_LEN: usize = 256;
const LABEL_PALETTE: [[f64; 3]; 12] = [
    [0.0, 0.447, 0.741],
    [0.850, 0.325, 0.098],
    [0.929, 0.694, 0.125],
    [0.494, 0.184, 0.556],
    [0.466, 0.674, 0.188],
    [0.301, 0.745, 0.933],
    [0.635, 0.078, 0.184],
    [0.950, 0.550, 0.100],
    [0.000, 0.620, 0.450],
    [0.800, 0.475, 0.655],
    [0.500, 0.800, 0.200],
    [0.950, 0.950, 0.950],
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum AlphaPreset {
    Linear,
    Ramp,
    Flat,
}

impl AlphaPreset {
    fn points(self) -> Vec<[f64; 2]> {
        match self {
            Self::Linear => vec![[0.0, 0.0], [1.0, 1.0]],
            Self::Ramp => vec![[0.0, 0.0], [0.3, 0.0], [0.7, 1.0], [1.0, 1.0]],
            Self::Flat => vec![[0.0, 0.5], [1.0, 0.5]],
        }
    }
}

fn default_alphamap(len: usize) -> Vec<f64> {
    if len <= 1 {
        return vec![1.0; len];
    }
    (0..len).map(|i| i as f64 / (len - 1) as f64).collect()
}

fn default_overlay_colormap() -> Arc<Vec<[f64; 3]>> {
    VolumeObject::new([1, 1, 1], Arc::from([0.0]), ImageDataClass::Double).overlay_colormap
}

const DEFAULT_OVERLAY_ALPHA: f64 = 0.5;

fn sample_alphamap(points: &[[f64; 2]]) -> [f64; ALPHAMAP_LEN] {
    let mut out = [0.0; ALPHAMAP_LEN];
    if points.is_empty() {
        return out;
    }
    for (i, slot) in out.iter_mut().enumerate() {
        let x = i as f64 / (ALPHAMAP_LEN - 1) as f64;
        let y = if x <= points[0][0] {
            points[0][1]
        } else if let Some(last) = points.last()
            && x >= last[0]
        {
            last[1]
        } else {
            let idx = points.partition_point(|p| p[0] <= x).saturating_sub(1);
            let p0 = points[idx];
            let p1 = points[(idx + 1).min(points.len() - 1)];
            if (p1[0] - p0[0]).abs() <= f64::EPSILON {
                p1[1]
            } else {
                let t = (x - p0[0]) / (p1[0] - p0[0]);
                p0[1] + t * (p1[1] - p0[1])
            }
        };
        *slot = y.clamp(0.0, 1.0);
    }
    out
}

fn compute_histogram(data: &[f64], limits: [f64; 2]) -> [f32; HISTOGRAM_BINS] {
    let [lo, hi] = limits;
    if !lo.is_finite() || !hi.is_finite() || hi <= lo {
        return [0.25; HISTOGRAM_BINS];
    }
    let mut counts = [0usize; HISTOGRAM_BINS];
    let span = hi - lo;
    let scale = (HISTOGRAM_BINS - 1) as f64;
    for &v in data {
        if v.is_finite() {
            let bin = (((v - lo) / span) * scale).round().clamp(0.0, scale) as usize;
            counts[bin] += 1;
        }
    }
    let max_count = counts.iter().copied().max().unwrap_or(0);
    let max_log = (1.0 + max_count as f64).ln().max(1.0);
    let mut out = [0.0f32; HISTOGRAM_BINS];
    for (dst, count) in out.iter_mut().zip(counts) {
        *dst = ((1.0 + count as f64).ln() / max_log) as f32;
    }
    out
}

struct Loaded {
    array: Array,
    volume: VolumeObject,
    name: String,
    sample: bool,
    spacing: [f64; 3],
    histogram: [f32; HISTOGRAM_BINS],
}

#[derive(Clone)]
struct LabelEntry {
    value: usize,
    name: String,
    count: usize,
    color: [f64; 3],
    opacity: f64,
    visible: bool,
}

struct LoadedLabels {
    array: Array,
    overlay: OverlayVolume,
    name: String,
    sample: bool,
    entries: Vec<LabelEntry>,
    max_label: usize,
    revision: u64,
}

impl LoadedLabels {
    fn render_settings(&self) -> OverlayRenderSettings {
        let len = self.max_label.clamp(1, MAX_COLORMAP_ROWS);
        let mut colormap = (*default_overlay_colormap()).clone();
        colormap.resize(len, [0.5, 0.5, 0.5]);
        let mut alphamap = vec![0.0; len];
        for entry in &self.entries {
            if (1..=len).contains(&entry.value) {
                let idx = entry.value - 1;
                colormap[idx] = entry.color;
                alphamap[idx] = if entry.visible {
                    entry.opacity.clamp(0.0, 1.0)
                } else {
                    0.0
                };
            }
        }
        OverlayRenderSettings {
            revision: self.revision,
            colormap,
            alphamap,
        }
    }
}

#[derive(Clone, PartialEq)]
struct OverlayRenderSettings {
    revision: u64,
    colormap: Vec<[f64; 3]>,
    alphamap: Vec<f64>,
}

#[derive(Clone, PartialEq)]
struct ViewSettings {
    style: RenderingStyle,
    cmap: &'static str,
    iso: f64,
    scale: [f64; 3],
    angles: [f64; 2],
    zoom: f64,
    background: [u8; 3],
    alphamap: [f64; ALPHAMAP_LEN],
    overlay: Option<OverlayRenderSettings>,
}

impl Default for ViewSettings {
    fn default() -> Self {
        let mut alphamap = [0.0; ALPHAMAP_LEN];
        alphamap.copy_from_slice(&default_alphamap(ALPHAMAP_LEN));
        Self {
            style: RenderingStyle::VolumeRendering,
            cmap: "gray",
            iso: 0.5,
            scale: [1.0; 3],
            angles: view3d::VIEW_3D,
            zoom: 1.0,
            background: [27, 30, 36],
            alphamap,
            overlay: None,
        }
    }
}

fn format_alphamap(alphamap: &[f64; ALPHAMAP_LEN]) -> String {
    if alphamap.iter().all(|&v| v == 0.0) {
        return "zeros(1,256)".into();
    }
    let linear = default_alphamap(ALPHAMAP_LEN);
    if alphamap.iter().zip(linear).all(|(&a, b)| a == b) {
        return "linspace(0,1,256)".into();
    }
    let values = alphamap
        .iter()
        .copied()
        .map(number)
        .collect::<Vec<_>>()
        .join(" ");
    format!("[{values}]")
}

impl ViewSettings {
    fn struct_expression(&self) -> String {
        let style = self.style.as_str();
        let [x, y, z] = self.scale.map(number);
        let [az, el] = self.angles.map(number);
        let [r, g, b] = self.background.map(|v| number(f64::from(v) / 255.0));
        let alpha_expr = format_alphamap(&self.alphamap);
        let overlay_fields = if let Some(ov) = &self.overlay {
            let cmap_rows = ov
                .colormap
                .iter()
                .map(|[cr, cg, cb]| format!("{} {} {}", number(*cr), number(*cg), number(*cb)))
                .collect::<Vec<_>>()
                .join("; ");
            let amap_vals = ov
                .alphamap
                .iter()
                .copied()
                .map(number)
                .collect::<Vec<_>>()
                .join(" ");
            format!(
                ", 'OverlayRenderingStyle', 'LabelOverlay', 'OverlayColormap', [{cmap_rows}], 'OverlayAlphamap', [{amap_vals}]"
            )
        } else {
            String::new()
        };
        format!(
            "struct('RenderingStyle', '{style}', 'Colormap', {}(256), \
             'Alphamap', {alpha_expr}, 'IsosurfaceValue', {}, 'ScaleFactors', [{x} {y} {z}], \
             'CameraPosition', [{az} {el}], 'Zoom', {}, 'BackgroundColor', [{r} {g} {b}]{overlay_fields})",
            self.cmap,
            number(self.iso),
            number(self.zoom),
        )
    }

    fn viewport(&self, volume: &VolumeObject) -> Viewport {
        let [rows, cols, _] = volume.dims;
        // Sample the axial projection at voxel centres when it fits the viewport.
        let (width, height) =
            if self.angles == [0.0, 90.0] && rows.max(cols) <= MAX_VIEW_PIXELS_PER_EDGE {
                (cols, rows)
            } else {
                display_render_size(volume, self.angles, MAX_VIEW_PIXELS_PER_EDGE)
            };
        Viewport {
            view: self.angles,
            size: [
                width.min(MAX_VIEW_PIXELS_PER_EDGE),
                height.min(MAX_VIEW_PIXELS_PER_EDGE),
            ],
            zoom: self.zoom,
            background: self.background,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
struct SliceSettings {
    numbers: [usize; 3],
    cmap: &'static str,
    scale: [f64; 3],
}

struct Rendered {
    settings: ViewSettings,
    viewport: Viewport,
    pixels: Vec<[u8; 4]>,
}

struct SlicePixels {
    size: [usize; 2],
    aspect: f32,
    rgb: Vec<u8>,
}

#[derive(Clone, Copy, PartialEq)]
enum JobKind {
    Load,
    LoadLabels,
    Scan,
    Render,
    Slices,
}

enum Prepared {
    Loaded(Box<Loaded>),
    Labels(Box<LoadedLabels>),
    Series(dicom::SeriesList),
    Views(Box<PreparedViews>),
}

struct PreparedViews {
    render: Option<Rendered>,
    slices: Option<(SliceSettings, [SlicePixels; 3])>,
}

struct Pending {
    kind: JobKind,
    payload: Arc<Mutex<Option<Prepared>>>,
}

#[derive(Default, PartialEq)]
enum Import {
    #[default]
    None,
    Workspace,
    Dicom,
}

pub(crate) struct VolumeViewer {
    launch: Option<Launch>,
    loaded: Option<Box<Loaded>>,
    labels: Option<Box<LoadedLabels>>,
    labels_only: bool,
    pending: Option<Pending>,
    serial: u64,
    label_revision: u64,
    requested_view: Option<(ViewSettings, SliceSettings)>,
    view: ViewSettings,
    alpha_points: Vec<[f64; 2]>,
    alpha_selected: Option<usize>,
    alpha_dragging: Option<usize>,
    numbers: [usize; 3],
    render: Option<Rendered>,
    texture: Option<egui::TextureHandle>,
    slice_settings: Option<SliceSettings>,
    slice_textures: Option<[(egui::TextureHandle, f32); 3]>,
    import: Import,
    import_error: Option<String>,
    label_import_error: Option<String>,
    workspace_name: String,
    label_workspace_name: String,
    label_picker_open: Option<usize>,
    folder: String,
    browse_requested: bool,
    series: Option<dicom::SeriesList>,
    series_row: usize,
    export_name: String,
    export_name_initialized: bool,
    #[cfg(test)]
    pub(super) controls: std::collections::BTreeMap<String, egui::Rect>,
    #[cfg(test)]
    before_render: Option<Box<dyn FnOnce() + Send>>,
}

impl Default for VolumeViewer {
    fn default() -> Self {
        Self {
            launch: None,
            loaded: None,
            labels: None,
            labels_only: false,
            pending: None,
            serial: 0,
            label_revision: 0,
            requested_view: None,
            view: ViewSettings::default(),
            alpha_points: AlphaPreset::Linear.points(),
            alpha_selected: None,
            alpha_dragging: None,
            numbers: [0; 3],
            render: None,
            texture: None,
            slice_settings: None,
            slice_textures: None,
            import: Import::default(),
            import_error: None,
            label_import_error: None,
            workspace_name: String::new(),
            label_workspace_name: String::new(),
            label_picker_open: None,
            folder: String::new(),
            browse_requested: false,
            series: None,
            series_row: 0,
            export_name: String::new(),
            export_name_initialized: false,
            #[cfg(test)]
            controls: std::collections::BTreeMap::new(),
            #[cfg(test)]
            before_render: None,
        }
    }
}

impl VolumeViewer {
    fn note(&mut self, label: &str, response: &egui::Response) {
        #[cfg(test)]
        self.controls.insert(label.to_owned(), response.rect);
        #[cfg(not(test))]
        let _ = (label, response);
    }

    fn effective_view(&self) -> ViewSettings {
        let mut view = self.view.clone();
        if self.labels_only {
            // Only the volume compositor applies base opacity along each label ray.
            // The style picker retains the base style for when labels-only is cleared.
            view.style = RenderingStyle::VolumeRendering;
            view.alphamap.fill(0.0);
        } else {
            view.alphamap = sample_alphamap(&self.alpha_points);
        }
        view.overlay = self.labels.as_ref().map(|labels| labels.render_settings());
        view
    }

    fn slice_request(&self) -> SliceSettings {
        SliceSettings {
            numbers: self.numbers,
            cmap: self.view.cmap,
            scale: self.view.scale,
        }
    }

    fn status_text(&self) -> Option<&'static str> {
        match self.pending.as_ref()?.kind {
            JobKind::Load => Some("Loading volume…"),
            JobKind::LoadLabels => Some("Loading labels…"),
            JobKind::Scan => Some("Scanning DICOM folder…"),
            JobKind::Render => Some("Rendering…"),
            JobKind::Slices => Some("Updating slices…"),
        }
    }

    fn current_render(&self) -> Option<&Rendered> {
        self.render
            .as_ref()
            .filter(|r| r.settings == self.effective_view())
    }

    fn settings_name(&self) -> Result<String> {
        let name = self.export_name.trim();
        super::check_export_name(name)?;
        if !matrust::files::valid_name(name) || matrust::session::FUNCTIONS.contains(&name) {
            return Err("Choose a valid workspace variable name".into());
        }
        matrust::syntax::parse(&format!("{name} = [];"))?;
        Ok(name.to_owned())
    }

    fn receive(&mut self, prepared: Prepared, ctx: &egui::Context) {
        match prepared {
            Prepared::Loaded(data) => {
                self.view.scale = data.spacing;
                let [lo, hi] = data.volume.data_limits;
                self.view.iso = lo * 0.5 + hi * 0.5;
                self.view.angles = view3d::VIEW_3D;
                self.view.zoom = 1.0;
                self.numbers = PLANES.map(|(_, axis, _)| axis.default_slice(data.volume.dims));
                self.loaded = Some(data);
                self.labels = None;
                self.labels_only = false;
                self.label_import_error = None;
                self.label_picker_open = None;
                self.requested_view = None;
                self.render = None;
                self.texture = None;
                self.slice_settings = None;
                self.slice_textures = None;
            }
            Prepared::Labels(labels) => {
                self.labels = Some(labels);
                self.label_import_error = None;
                self.label_picker_open = None;
                self.requested_view = None;
            }
            Prepared::Series(collection) => {
                self.series = Some(collection);
                self.series_row = 0;
            }
            Prepared::Views(views) => {
                let PreparedViews { render, slices } = *views;
                if let Some(render) = render {
                    let image = egui::ColorImage::from_rgba_unmultiplied(
                        render.viewport.size,
                        render.pixels.as_flattened(),
                    );
                    self.texture = Some(ctx.load_texture(
                        "Volume Viewer 3-D",
                        image,
                        egui::TextureOptions::LINEAR,
                    ));
                    self.render = Some(render);
                }
                if let Some((settings, slices)) = slices {
                    self.slice_textures = Some(slices.map(|slice| {
                        let image = egui::ColorImage::from_rgb(slice.size, &slice.rgb);
                        (
                            ctx.load_texture(
                                "Volume Viewer slice",
                                image,
                                egui::TextureOptions::NEAREST,
                            ),
                            slice.aspect,
                        )
                    }));
                    self.slice_settings = Some(settings);
                }
            }
        }
    }

    fn rendering_controls(&mut self, ui: &mut egui::Ui) {
        ui.add_space(tokens::M);
        ui.add_enabled_ui(self.loaded.is_some(), |ui| {
            let label = STYLES
                .iter()
                .find(|(style, _)| *style == self.view.style)
                .unwrap()
                .1;
            ui.label("Rendering style");
            egui::ComboBox::from_id_salt("volume_style")
                .selected_text(label)
                .show_ui(ui, |ui| {
                    for (value, label) in STYLES {
                        ui.selectable_value(&mut self.view.style, value, label);
                    }
                });
            if self.view.style == RenderingStyle::Isosurface
                && let Some(data) = &self.loaded
            {
                let [lo, hi] = data.volume.data_limits;
                ui.label("Isosurface value");
                ui.add_enabled(lo < hi, egui::Slider::new(&mut self.view.iso, lo..=hi));
            }
            ui.label("Colormap");
            egui::ComboBox::from_id_salt("volume_colormap")
                .selected_text(self.view.cmap)
                .show_ui(ui, |ui| {
                    for name in COLORMAPS {
                        ui.selectable_value(&mut self.view.cmap, name, name);
                    }
                });
            ui.horizontal(|ui| {
                ui.label("Background");
                tokens::color_edit_button(ui, &mut self.view.background);
            });
            ui.add_space(tokens::S);
            ui.label("Voxel sizes");
            egui::Grid::new("volume_scale").show(ui, |ui| {
                for (label, scale) in ["X", "Y", "Z"].into_iter().zip(&mut self.view.scale) {
                    ui.label(label);
                    let speed = (*scale * 0.02).max(1e-6);
                    ui.add(egui::DragValue::new(scale).range(SPACING).speed(speed));
                    ui.end_row();
                }
            });
            ui.add_space(tokens::M);
            self.alphamap_editor(ui);
        });
    }

    fn alphamap_editor(&mut self, ui: &mut egui::Ui) {
        let p = tokens::palette(ui.ctx());
        ui.label("Alphamap");
        ui.horizontal(|ui| {
            for (preset, label, key) in [
                (AlphaPreset::Linear, "Linear", "Preset Linear"),
                (AlphaPreset::Ramp, "Ramp", "Preset Ramp"),
                (AlphaPreset::Flat, "Flat", "Preset Flat"),
            ] {
                let r = ui.button(label);
                self.note(key, &r);
                if r.clicked() {
                    self.alpha_points = preset.points();
                    self.alpha_selected = None;
                    self.alpha_dragging = None;
                }
            }
        });
        let size = egui::vec2(ui.available_width().max(1.0), 96.0);
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
        self.note("Alphamap editor", &response);
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, p.surface);
        painter.rect_stroke(
            rect,
            0.0,
            egui::Stroke::new(1.0, p.border),
            egui::StrokeKind::Inside,
        );
        let plot = rect.shrink(6.0);
        if let Some(data) = &self.loaded {
            let bin_w = plot.width() / HISTOGRAM_BINS as f32;
            let bar_color = egui::Color32::from_gray(120);
            for (i, &h) in data.histogram.iter().enumerate() {
                if h > 0.0 {
                    let x0 = plot.left() + i as f32 * bin_w;
                    let x1 = x0 + bin_w.max(1.0);
                    let y0 = plot.bottom() - h * plot.height();
                    painter.rect_filled(
                        egui::Rect::from_min_max(egui::pos2(x0, y0), egui::pos2(x1, plot.bottom())),
                        0.0,
                        bar_color,
                    );
                }
            }
        }
        let to_screen = |pt: [f64; 2]| {
            egui::pos2(
                plot.left() + pt[0] as f32 * plot.width(),
                plot.bottom() - pt[1] as f32 * plot.height(),
            )
        };
        let to_norm = |pos: egui::Pos2| {
            [
                f64::from((pos.x - plot.left()) / plot.width().max(1.0)).clamp(0.0, 1.0),
                f64::from((plot.bottom() - pos.y) / plot.height().max(1.0)).clamp(0.0, 1.0),
            ]
        };

        if self.loaded.is_some() {
            const HIT_RADIUS: f32 = 10.0;
            let nearest = |pos: egui::Pos2, points: &[[f64; 2]]| {
                points
                    .iter()
                    .enumerate()
                    .map(|(i, &pt)| (i, to_screen(pt).distance(pos)))
                    .filter(|&(_, d)| d <= HIT_RADIUS)
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                    .map(|(i, _)| i)
            };
            if response.drag_started()
                && let Some(pos) = ui
                    .input(|i| i.pointer.press_origin())
                    .or_else(|| response.interact_pointer_pos())
            {
                self.alpha_dragging = nearest(pos, &self.alpha_points);
                if self.alpha_dragging.is_some() {
                    self.alpha_selected = self.alpha_dragging;
                }
            }
            if response.dragged()
                && let Some(idx) = self.alpha_dragging
                && let Some(pos) = response.interact_pointer_pos()
            {
                let [raw_x, y] = to_norm(pos);
                let last = self.alpha_points.len() - 1;
                let x = if idx == 0 {
                    0.0
                } else if idx == last {
                    1.0
                } else {
                    raw_x.clamp(self.alpha_points[idx - 1][0], self.alpha_points[idx + 1][0])
                };
                self.alpha_points[idx] = [x, y];
            }
            if response.drag_stopped() {
                self.alpha_dragging = None;
            }
            if response.clicked()
                && let Some(pos) = response.interact_pointer_pos()
            {
                if let Some(idx) = nearest(pos, &self.alpha_points) {
                    self.alpha_selected = Some(idx);
                } else {
                    let [raw_x, y] = to_norm(pos);
                    let x = raw_x.clamp(1e-4, 1.0 - 1e-4);
                    let insert_at = self.alpha_points.partition_point(|pt| pt[0] < x);
                    let clamped_at = insert_at.clamp(1, self.alpha_points.len() - 1);
                    self.alpha_points.insert(clamped_at, [x, y]);
                    self.alpha_selected = Some(clamped_at);
                }
            }
            if response.secondary_clicked()
                && let Some(pos) = response.interact_pointer_pos()
                && let Some(idx) = nearest(pos, &self.alpha_points)
                && idx > 0
                && idx + 1 < self.alpha_points.len()
            {
                self.alpha_points.remove(idx);
                self.alpha_selected = None;
                self.alpha_dragging = None;
            }
            if !ui.ctx().egui_wants_keyboard_input()
                && ui.input(|i| {
                    i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace)
                })
                && let Some(idx) = self.alpha_selected
                && idx > 0
                && idx + 1 < self.alpha_points.len()
            {
                self.alpha_points.remove(idx);
                self.alpha_selected = None;
                self.alpha_dragging = None;
            }
        }

        let curve_color = if self.loaded.is_some() {
            p.accent
        } else {
            p.text_disabled
        };
        let screen_pts: Vec<egui::Pos2> =
            self.alpha_points.iter().copied().map(to_screen).collect();
        if screen_pts.len() >= 2 {
            painter.add(egui::Shape::line(
                screen_pts.clone(),
                egui::Stroke::new(1.5, curve_color),
            ));
        }
        for (i, pos) in screen_pts.into_iter().enumerate() {
            let selected = self.alpha_selected == Some(i);
            let fill = if selected { curve_color } else { p.surface };
            painter.circle(
                pos,
                if selected { 4.5 } else { 3.5 },
                fill,
                egui::Stroke::new(1.5, curve_color),
            );
        }

        ui.add_space(tokens::XS);
        let strip_size = egui::vec2(ui.available_width().max(1.0), 14.0);
        let (strip_rect, _) = ui.allocate_exact_size(strip_size, egui::Sense::hover());
        let strip_painter = ui.painter_at(strip_rect);
        if let Ok(Some(table)) = colormap::named(self.view.cmap, 64) {
            let seg_w = strip_rect.width() / table.len() as f32;
            for (i, [r, g, b]) in table.into_iter().enumerate() {
                let x0 = strip_rect.left() + i as f32 * seg_w;
                let x1 = x0 + seg_w + 0.5;
                let color = egui::Color32::from_rgb(
                    (r.clamp(0.0, 1.0) * 255.0).round() as u8,
                    (g.clamp(0.0, 1.0) * 255.0).round() as u8,
                    (b.clamp(0.0, 1.0) * 255.0).round() as u8,
                );
                strip_painter.rect_filled(
                    egui::Rect::from_min_max(
                        egui::pos2(x0, strip_rect.top()),
                        egui::pos2(x1, strip_rect.bottom()),
                    ),
                    0.0,
                    color,
                );
            }
        }
        strip_painter.rect_stroke(
            strip_rect,
            0.0,
            egui::Stroke::new(1.0, p.border),
            egui::StrokeKind::Inside,
        );
    }

    fn labels_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) -> (Option<(String, Array)>, bool) {
        let mut load_ws_labels = None;
        let mut use_sample_labels = false;
        let p = tokens::palette(ui.ctx());
        ui.add_space(tokens::M);
        ui.label("Labels");
        ui.add_enabled_ui(self.loaded.is_some(), |ui| {
            let dims = self.loaded.as_ref().map(|d| d.volume.dims);
            egui::ComboBox::from_id_salt("volume_import_labels")
                .width(ui.available_width())
                .wrap()
                .selected_text(if self.label_workspace_name.is_empty() {
                    "Import labels"
                } else {
                    &self.label_workspace_name
                })
                .show_ui(ui, |ui| {
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                    let Some(dims) = dims else { return };
                    let mut entries = workspace_label_volumes(ws, dims).peekable();
                    if entries.peek().is_none() {
                        tokens::helper(
                            ui,
                            "No matching integer, logical, or categorical variables",
                        );
                    }
                    for (name, array) in entries {
                        ui.selectable_value(
                            &mut self.label_workspace_name,
                            name.to_owned(),
                            format!("{name}  {}", array.class()),
                        );
                    }
                });
            ui.horizontal_wrapped(|ui| {
                let ready = dims.is_some_and(|d| {
                    ws.get(&self.label_workspace_name)
                        .is_some_and(|a| is_valid_label_array(a, d))
                });
                let r = ui.add_enabled(ready, egui::Button::new("Load labels"));
                self.note("Load labels", &r);
                if r.clicked()
                    && let Some(array) = ws.get(&self.label_workspace_name)
                {
                    load_ws_labels = Some((self.label_workspace_name.clone(), array.clone()));
                }
                let can_sample = self
                    .loaded
                    .as_ref()
                    .is_some_and(|d| d.sample || d.volume.dims == [96, 96, 96]);
                let r = ui.add_enabled(can_sample, egui::Button::new("Use sample labels"));
                self.note("Use sample labels", &r);
                if r.clicked() {
                    use_sample_labels = true;
                }
                let r = ui.add_enabled(self.labels.is_some(), egui::Button::new("Clear labels"));
                self.note("Clear labels", &r);
                if r.clicked() {
                    self.labels = None;
                    self.labels_only = false;
                    self.label_import_error = None;
                    self.label_picker_open = None;
                }
            });
            if let Some(err) = &self.label_import_error {
                ui.label(egui::RichText::new(err).color(p.error));
            }
            let r = ui.add_enabled(
                self.labels.is_some(),
                egui::Checkbox::new(&mut self.labels_only, "Labels only"),
            );
            self.note("Labels only", &r);
            if let Some(labels) = &mut self.labels {
                ui.add_space(tokens::XS);
                if labels.entries.is_empty() {
                    tokens::helper(ui, "No positive labels in volume");
                } else {
                    egui::ScrollArea::vertical()
                        .id_salt("volume_labels_table")
                        .max_height(220.0)
                        .show_themed(ui, |ui| {
                            ui.spacing_mut().slider_width = 56.0;
                            for entry in &mut labels.entries {
                                let swatch = egui::Color32::from_rgb(
                                    (entry.color[0].clamp(0.0, 1.0) * 255.0).round() as u8,
                                    (entry.color[1].clamp(0.0, 1.0) * 255.0).round() as u8,
                                    (entry.color[2].clamp(0.0, 1.0) * 255.0).round() as u8,
                                );
                                ui.horizontal_wrapped(|ui| {
                                    ui.checkbox(&mut entry.visible, "");
                                    let btn = ui.add(
                                        egui::Button::new("")
                                            .fill(swatch)
                                            .stroke(tokens::swatch_stroke(ui))
                                            .min_size(egui::vec2(16.0, 16.0)),
                                    );
                                    if btn.clicked() {
                                        self.label_picker_open =
                                            if self.label_picker_open == Some(entry.value) {
                                                None
                                            } else {
                                                Some(entry.value)
                                            };
                                    }
                                    // Wrap the name beside its checkbox; remaining controls
                                    // can flow to the next line within this same row.
                                    ui.vertical(|ui| ui.label(&entry.name));
                                    tokens::helper(ui, format!("{}", entry.count));
                                    ui.add(
                                        egui::Slider::new(&mut entry.opacity, 0.0..=1.0)
                                            .show_value(false),
                                    );
                                });
                                if self.label_picker_open == Some(entry.value) {
                                    ui.horizontal_wrapped(|ui| {
                                        for &rgb in &LABEL_PALETTE {
                                            let c32 = egui::Color32::from_rgb(
                                                (rgb[0] * 255.0).round() as u8,
                                                (rgb[1] * 255.0).round() as u8,
                                                (rgb[2] * 255.0).round() as u8,
                                            );
                                            if ui
                                                .add(
                                                    egui::Button::new("")
                                                        .fill(c32)
                                                        .stroke(tokens::swatch_stroke(ui))
                                                        .min_size(egui::vec2(14.0, 14.0)),
                                                )
                                                .clicked()
                                            {
                                                entry.color = rgb;
                                                self.label_picker_open = None;
                                            }
                                        }
                                    });
                                }
                            }
                        });
                }
            }
        });
        (load_ws_labels, use_sample_labels)
    }

    fn slices_ui(&mut self, ui: &mut egui::Ui) {
        for (i, (label, direction, _)) in PLANES.into_iter().enumerate() {
            ui.label(label);
            let Some(data) = &self.loaded else {
                tokens::helper(ui, "No slice");
                ui.add_space(tokens::M);
                continue;
            };
            let limit = direction.axis_len(data.volume.dims);
            let response = ui
                .add_enabled_ui(limit > 1, |ui| {
                    let width = ui.available_width();
                    let spacing = ui.spacing();
                    let stacked = width
                        < spacing.slider_width + spacing.item_spacing.x + spacing.interact_size.x;
                    if stacked {
                        ui.spacing_mut().slider_width = width.max(1.0);
                    }
                    let number = &mut self.numbers[i];
                    let slider = ui.add(
                        egui::Slider::new(number, 1..=limit)
                            .integer()
                            .show_value(!stacked),
                    );
                    if stacked {
                        slider | ui.add(egui::DragValue::new(number).range(1..=limit))
                    } else {
                        slider
                    }
                })
                .inner;
            self.note(label, &response);
            let size = egui::vec2(ui.available_width(), 115.0);
            let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
            if let Some(textures) = &self.slice_textures {
                let (texture, aspect) = &textures[i];
                paint_image(ui, rect, texture, *aspect);
            } else {
                ui.painter()
                    .rect_filled(rect, 0.0, tokens::palette(ui.ctx()).surface);
            }
            ui.add_space(tokens::S);
        }
    }
}

pub(super) fn workspace_volumes(ws: &Snapshot) -> impl Iterator<Item = (&str, &Array)> {
    ws.variables()
        .filter(|(_, array)| volume_shape(array).is_ok())
}

fn is_valid_label_array(array: &Array, dims: [usize; 3]) -> bool {
    array.dims().as_ref() == dims.as_slice()
        && match array {
            Array::Logical(_) | Array::Categorical(_) => true,
            Array::Typed(typed) => typed.class() != NumericClass::Single,
            _ => false,
        }
}

pub(super) fn workspace_label_volumes(
    ws: &Snapshot,
    dims: [usize; 3],
) -> impl Iterator<Item = (&str, &Array)> {
    ws.variables()
        .filter(move |(_, array)| is_valid_label_array(array, dims))
}

fn load_data(
    engine: &mut Engine,
    array: Array,
    name: String,
    sample: bool,
    spacing: [f64; 3],
) -> Result<Prepared> {
    if !spacing.iter().all(|value| SPACING.contains(value)) {
        return Err("Voxel sizes must be between 1e-6 and 1e6".into());
    }
    let volume = prepare_volume(&array, &engine.cancellation())?;
    let histogram = compute_histogram(&volume.data, volume.data_limits);
    Ok(Prepared::Loaded(Box::new(Loaded {
        array,
        volume,
        name,
        sample,
        spacing,
        histogram,
    })))
}

fn prepare_labels(
    array: Array,
    name: String,
    sample: bool,
    dims: [usize; 3],
    revision: u64,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<Prepared> {
    if !is_valid_label_array(&array, dims) {
        return Err(
            "Labels must be an integer, logical, or categorical array of the same size".into(),
        );
    }
    matrust::dsp::check_cancel(cancel)?;
    let (data, class): (Arc<[f64]>, OverlayDataClass) = match &array {
        Array::Logical(m) => {
            let vals: Vec<f64> = m.data().iter().map(|&b| f64::from(b)).collect();
            (Arc::from(vals), OverlayDataClass::Logical)
        }
        Array::Typed(t) => {
            let vals: Vec<f64> = (0..t.len()).map(|i| t.real(i)).collect();
            (Arc::from(vals), OverlayDataClass::Typed(t.class()))
        }
        Array::Categorical(c) => {
            let vals: Vec<f64> = c.codes().iter().map(|&code| f64::from(code)).collect();
            (
                Arc::from(vals),
                OverlayDataClass::Categorical {
                    categories: c.categories().clone(),
                    ordinal: c.is_ordinal(),
                },
            )
        }
        _ => unreachable!(),
    };
    matrust::dsp::check_cancel(cancel)?;
    let mut counts = std::collections::BTreeMap::<usize, usize>::new();
    for (idx, &v) in data.iter().enumerate() {
        if idx & 4095 == 0 {
            matrust::dsp::check_cancel(cancel)?;
        }
        if v.is_finite() && v > 0.0 {
            let k = v.round() as usize;
            if k > MAX_COLORMAP_ROWS {
                return Err(format!(
                    "Label value {k} exceeds {MAX_COLORMAP_ROWS} maximum"
                ));
            }
            if k > 0 {
                *counts.entry(k).or_insert(0) += 1;
            }
        }
    }
    let palette = default_overlay_colormap();
    let max_label = counts.keys().next_back().copied().unwrap_or(1);
    let entries = counts
        .into_iter()
        .map(|(value, count)| {
            let label_name = match &class {
                OverlayDataClass::Categorical { categories, .. } => categories
                    .get(value - 1)
                    .cloned()
                    .unwrap_or_else(|| value.to_string()),
                _ => value.to_string(),
            };
            LabelEntry {
                value,
                name: label_name,
                count,
                color: palette[(value - 1) % palette.len()],
                opacity: DEFAULT_OVERLAY_ALPHA,
                visible: true,
            }
        })
        .collect();
    Ok(Prepared::Labels(Box::new(LoadedLabels {
        array,
        overlay: OverlayVolume { data, class },
        name,
        sample,
        entries,
        max_label,
        revision,
    })))
}

pub(super) fn load_sample(engine: &mut Engine) -> Result<Array> {
    engine.execute(&format!("vv_volume = vv_shepp_logan();\n{SAMPLE}"))?;
    engine
        .array("vv_volume")
        .ok_or_else(|| "The sample did not create a volume".into())
}

fn load_sample_labels(engine: &mut Engine) -> Result<Array> {
    engine.execute(&format!("[~, vv_labels] = vv_shepp_logan();\n{SAMPLE}"))?;
    engine
        .array("vv_labels")
        .ok_or_else(|| "The sample did not create labels".into())
}

fn submit(
    host: &mut Host<VolumeViewer>,
    ctx: &egui::Context,
    kind: JobKind,
    task: impl FnOnce(&mut Engine) -> Result<Prepared> + Send + 'static,
) {
    let payload = Arc::new(Mutex::new(None));
    if matches!(kind, JobKind::Load | JobKind::LoadLabels | JobKind::Scan) {
        // An import retires the in-flight views even if validation later fails.
        host.app.requested_view = None;
        if kind == JobKind::LoadLabels {
            host.app.label_import_error = None;
        } else {
            host.app.import_error = None;
        }
    }
    host.app.pending = Some(Pending {
        kind,
        payload: payload.clone(),
    });
    host.app.serial = host.app.serial.wrapping_add(1);
    host.run_error = None;
    host.message.clear();
    host.runner.submit_task(
        ctx,
        host.app.serial,
        Box::new(move |engine| {
            let prepared = task(engine)?;
            *payload
                .lock()
                .map_err(|_| "Volume Viewer result lock failed")? = Some(prepared);
            Ok(Outputs::new())
        }),
    );
}

fn submit_array(host: &mut Host<VolumeViewer>, ctx: &egui::Context, array: Array, name: String) {
    submit(host, ctx, JobKind::Load, move |engine| {
        load_data(engine, array, name, false, [1.0; 3])
    });
}

fn submit_sample(host: &mut Host<VolumeViewer>, ctx: &egui::Context) {
    submit(host, ctx, JobKind::Load, |engine| {
        let array = load_sample(engine)?;
        load_data(engine, array, "Shepp–Logan phantom".into(), true, [1.0; 3])
    });
}

fn submit_labels(
    host: &mut Host<VolumeViewer>,
    ctx: &egui::Context,
    array: Array,
    name: String,
    dims: [usize; 3],
) {
    host.app.label_revision = host.app.label_revision.wrapping_add(1);
    let rev = host.app.label_revision;
    submit(host, ctx, JobKind::LoadLabels, move |engine| {
        prepare_labels(array, name, false, dims, rev, &engine.cancellation())
    });
}

fn submit_sample_labels(host: &mut Host<VolumeViewer>, ctx: &egui::Context, dims: [usize; 3]) {
    host.app.label_revision = host.app.label_revision.wrapping_add(1);
    let rev = host.app.label_revision;
    submit(host, ctx, JobKind::LoadLabels, move |engine| {
        let array = load_sample_labels(engine)?;
        prepare_labels(
            array,
            "vv_labels".into(),
            true,
            dims,
            rev,
            &engine.cancellation(),
        )
    });
}

pub(super) fn scan_folder(host: &mut Host<VolumeViewer>, ctx: &egui::Context, folder: PathBuf) {
    host.app.series = None;
    submit(host, ctx, JobKind::Scan, move |engine| {
        dicom::scan(engine, &folder).map(Prepared::Series)
    });
}

fn submit_series(host: &mut Host<VolumeViewer>, ctx: &egui::Context) -> Result<()> {
    let collection = host
        .app
        .series
        .as_ref()
        .ok_or("Scan a DICOM folder first")?;
    let row = host.app.series_row;
    let series = collection.rows.get(row).ok_or("Select a DICOM series")?;
    let dims = [series.rows, series.cols, series.frames];
    let name = format!("DICOM {}", series.uid);
    let files = dicom::series_files(collection, row)?;
    submit(host, ctx, JobKind::Load, move |engine| {
        let (array, spacing) = dicom::load(engine, files, dims)?;
        load_data(engine, array, name, false, spacing)
    });
    Ok(())
}

fn slice_pixels(volume: &VolumeObject, settings: SliceSettings, pane: usize) -> SlicePixels {
    let (_, axis, [x, y]) = PLANES[pane];
    let image = extract_slice_image(
        volume.dims,
        &volume.data,
        volume.data_class,
        axis,
        settings.numbers[pane],
        settings.scale,
        true,
    );
    SlicePixels {
        size: [image.cols, image.rows],
        aspect: ((image.cols as f64 * settings.scale[x]) / (image.rows as f64 * settings.scale[y]))
            as f32,
        rgb: image.rgb(&volume.colormap, volume.data_limits),
    }
}

fn submit_views(host: &mut Host<VolumeViewer>, ctx: &egui::Context) {
    let app = &mut host.app;
    let Some(data) = &app.loaded else { return };
    if app.pending.as_ref().is_some_and(|job| {
        matches!(
            job.kind,
            JobKind::Load | JobKind::LoadLabels | JobKind::Scan
        )
    }) {
        return;
    }
    let settings = app.effective_view();
    let slices = app.slice_request();
    if app.requested_view.as_ref() == Some(&(settings.clone(), slices)) {
        return;
    }
    app.requested_view = Some((settings.clone(), slices));
    let update_render = app
        .render
        .as_ref()
        .is_none_or(|render| render.settings != settings);
    let update_slices = app.slice_settings != Some(slices);
    if !update_render && !update_slices {
        host.runner.stop();
        app.pending = None;
        return;
    }
    let kind = if update_render {
        JobKind::Render
    } else {
        JobKind::Slices
    };
    let mut volume = data.volume.clone();
    let overlay_data = app.labels.as_ref().map(|labels| labels.overlay.clone());
    #[cfg(test)]
    let before_render = app.before_render.take();
    submit(host, ctx, kind, move |engine| {
        let cancel = engine.cancellation();
        volume.colormap = Arc::new(
            colormap::named(settings.cmap, colormap::DEFAULT_ROWS)?
                .ok_or("Unknown Volume Viewer colormap")?,
        );
        volume.alphamap = Arc::new(settings.alphamap.to_vec());
        volume.transformation = settings.scale;
        volume.rendering_style = settings.style;
        volume.isosurface_value = settings.iso;
        if let Some(ov) = &settings.overlay {
            volume.overlay_data = overlay_data;
            volume.overlay_rendering_style = OverlayRenderingStyle::LabelOverlay;
            volume.overlay_colormap = Arc::new(ov.colormap.clone());
            volume.overlay_alphamap = Arc::new(ov.alphamap.clone());
        } else {
            volume.overlay_data = None;
        }

        let slices = if update_slices {
            let mut images = Vec::with_capacity(3);
            for pane in 0..3 {
                matrust::dsp::check_cancel(&cancel)?;
                images.push(slice_pixels(&volume, slices, pane));
            }
            Some((slices, images.try_into().ok().unwrap()))
        } else {
            None
        };
        let render = if update_render {
            let viewport = settings.viewport(&volume);
            #[cfg(test)]
            if let Some(before_render) = before_render {
                before_render();
            }
            let pixels = viewport.render(&volume, &cancel)?;
            Some(Rendered {
                settings,
                viewport,
                pixels,
            })
        } else {
            None
        };
        Ok(Prepared::Views(Box::new(PreparedViews { render, slices })))
    });
}

fn refresh(host: &mut Host<VolumeViewer>, ctx: &egui::Context) {
    if let Some(Launch::Volume { name, array }) = host.app.launch.take() {
        submit_array(
            host,
            ctx,
            array,
            name.unwrap_or_else(|| "Command input".into()),
        );
    }
    host.runner.poll(ctx);
    if let Some(result) = host.runner.result()
        && let Some(pending) = host.app.pending.take()
    {
        if let Err(error) = result {
            let message = error.lines().next().unwrap_or(error).to_owned();
            if matches!(pending.kind, JobKind::Load | JobKind::Scan) {
                host.app.import_error = Some(message);
            } else if pending.kind == JobKind::LoadLabels {
                host.app.label_import_error = Some(message);
            } else {
                host.run_error = Some(message);
            }
        } else {
            let prepared = pending.payload.lock().ok().and_then(|mut slot| slot.take());
            match prepared {
                Some(prepared) => host.app.receive(prepared, ctx),
                None => host.run_error = Some("Volume Viewer worker returned no result".into()),
            }
        }
    }
    submit_views(host, ctx);
}

fn paint_image(ui: &egui::Ui, rect: egui::Rect, texture: &egui::TextureHandle, aspect: f32) {
    let width = rect.width().min(rect.height() * aspect);
    let height = rect.height().min(rect.width() / aspect);
    let image =
        egui::Rect::from_center_size(rect.center(), egui::vec2(width.max(1.0), height.max(1.0)));
    ui.painter().with_clip_rect(rect).image(
        texture.id(),
        image,
        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)),
        egui::Color32::WHITE,
    );
}

/// Store category text as MAT data. A positive code indexes both CategoryNames
/// and the overlay maps. Zero remains transparent (undefined).
pub(crate) fn portable_label_array(
    array: Array,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<Array> {
    let Array::Categorical(labels) = array else {
        return Ok(array);
    };
    // Label import already rejects occupied codes beyond the palette limit.
    let names = labels
        .categories()
        .iter()
        .take(MAX_COLORMAP_ROWS)
        .map(|name| {
            matrust::CharacterRow::from_text(name, cancel)
                .map(|row| Arc::new(Array::Character(row)))
        })
        .collect::<Result<Vec<_>>>()?;
    let names = matrust::CellArray::from_shared(names.len(), 1, names, cancel)?;
    let fields = vec![
        Arc::new(Array::Typed(labels.code_array()?)),
        Arc::new(Array::Cell(names)),
        Arc::new(Array::Logical(matrust::LogicalMatrix::scalar(
            labels.is_ordinal(),
        ))),
    ];
    Ok(matrust::StructArray::from_slots(
        1,
        1,
        vec!["Codes".into(), "CategoryNames".into(), "Ordinal".into()],
        fields,
        cancel,
    )?
    .into())
}

pub(crate) struct LabelSettingsExport {
    name: String,
    script: String,
    overlay: Array,
}

impl LabelSettingsExport {
    pub(crate) fn execute(self, engine: &mut Engine) -> Result<matrust::Execution> {
        super::execute_export_then(engine, &self.script, |candidate| {
            let Some(Array::Struct(settings)) = candidate.array(&self.name) else {
                return Err("Volume Viewer did not create a settings struct".into());
            };
            let settings =
                settings.with_field("OverlayData", self.overlay, &candidate.cancellation())?;
            candidate.insert_array(&self.name, settings.into())
        })
    }
}

fn labels_script_var(labels: &LoadedLabels) -> &str {
    let name = labels.name.as_str();
    if !labels.sample
        && !matches!(labels.array, Array::Categorical(_))
        && matrust::files::valid_name(name)
        && !matrust::session::FUNCTIONS.contains(&name)
        && !name.starts_with("vv_")
    {
        name
    } else {
        "vv_labels"
    }
}

impl App for VolumeViewer {
    fn kind(&self) -> AppKind {
        AppKind::VolumeViewer
    }

    fn script(&self, _: &Snapshot) -> Result<String> {
        let data = self.loaded.as_ref().ok_or("Load a volume first")?;
        let rendered = self.current_render().ok_or("Wait for the current render")?;
        let settings = rendered.settings.struct_expression();
        let [width, height] = rendered.viewport.size;
        let sample_labels = self.labels.as_ref().is_some_and(|l| l.sample);
        let load = match (data.sample, sample_labels) {
            (true, true) => "[vv_volume, vv_labels] = vv_shepp_logan();\n",
            (true, false) => "vv_volume = vv_shepp_logan();\n",
            (false, true) => "[~, vv_labels] = vv_shepp_logan();\n",
            (false, false) => "",
        };
        let local = if data.sample || sample_labels {
            SAMPLE
        } else {
            ""
        };
        let (overlay_args, legend) = if let Some(labels) = &self.labels {
            let var = labels_script_var(labels);
            let (field, legend) = if matches!(labels.array, Array::Categorical(_)) {
                (
                    ".Codes",
                    "% Positive codes index CategoryNames and the overlay maps. Zero is undefined.\n\
                 vv_settings.OverlayCategoryNames = vv_labels.CategoryNames;\n\
                 vv_settings.OverlayOrdinal = vv_labels.Ordinal;\n",
                )
            } else {
                ("", "")
            };
            let args = format!(
                ", 'OverlayData', {var}{field}, 'OverlayRenderingStyle', vv_settings.OverlayRenderingStyle, \
                 'OverlayColormap', vv_settings.OverlayColormap, 'OverlayAlphamap', vv_settings.OverlayAlphamap"
            );
            (args, legend)
        } else {
            (String::new(), "")
        };
        Ok(format!(
            "{load}vv_settings = {settings};\n\
             {legend}figure;\n\
             vv_transform = diag([vv_settings.ScaleFactors 1]);\n\
             vv_object = volshow(vv_volume, 'RenderingStyle', vv_settings.RenderingStyle, \
             'Colormap', vv_settings.Colormap, 'Alphamap', vv_settings.Alphamap, \
             'IsosurfaceValue', vv_settings.IsosurfaceValue, 'Transformation', vv_transform{overlay_args});\n\
             view(vv_settings.CameraPosition);\n\
             vv_dims = size(vv_volume);\n\
             vv_whd = vv_dims([2 1 3]);\n\
             vv_center = (vv_whd + 1) .* vv_settings.ScaleFactors / 2;\n\
             vv_half = vv_whd .* vv_settings.ScaleFactors / (2 * vv_settings.Zoom);\n\
             vv_limits = [vv_center - vv_half; vv_center + vv_half];\n\
             axis(vv_limits(:).'); axis off;\n\
             set(gcf, 'Color', vv_settings.BackgroundColor);\n\
             set(gca, 'Color', vv_settings.BackgroundColor);\n\
             % Export the Volume Viewer viewport at its original pixel dimensions.\n\
             vv_camera = [vv_settings.CameraPosition vv_settings.Zoom];\n\
             vv_rgb = builtin('volumeViewer', 'replayViewport', vv_object, vv_camera, [{width} {height}], vv_settings.BackgroundColor);\n\
             imwrite(vv_rgb, 'volume_viewer.png');\n{local}"
        ))
    }

    fn inputs(&self) -> Vec<String> {
        vec![]
    }
    fn probe(&self, _: &Snapshot) -> (String, Vec<String>) {
        (String::new(), vec![])
    }
    fn receive(&mut self, _: &Outputs) -> Result<()> {
        Ok(())
    }

    fn inputs_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        tokens::helper(
            ui,
            self.loaded
                .as_ref()
                .map_or("No volume loaded", |data| data.name.as_str()),
        );
        ui.add_space(tokens::M);
        match self.import {
            Import::None => {}
            Import::Workspace => {
                let caption = ui.label("Workspace volume");
                egui::ComboBox::from_id_salt("volume_import_workspace")
                    .width(ui.available_width())
                    .wrap()
                    .selected_text(if self.workspace_name.is_empty() {
                        "Select variable"
                    } else {
                        &self.workspace_name
                    })
                    .show_ui(ui, |ui| {
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                        let mut entries = workspace_volumes(ws).peekable();
                        if entries.peek().is_none() {
                            tokens::helper(ui, "No 3-D numeric or logical variables");
                        }
                        for (name, array) in entries {
                            let [rows, cols, pages] = *array.dims() else {
                                continue;
                            };
                            ui.selectable_value(
                                &mut self.workspace_name,
                                name.to_owned(),
                                format!("{name}  {rows} × {cols} × {pages}  {}", array.class()),
                            );
                        }
                    })
                    .response
                    .labelled_by(caption.id);
            }
            Import::Dicom => {
                ui.label("DICOM folder");
                let scanning = self
                    .pending
                    .as_ref()
                    .is_some_and(|job| job.kind == JobKind::Scan);
                let field =
                    tokens::input(egui::TextEdit::singleline(&mut self.folder).char_limit(4096));
                if ui.add_enabled(!scanning, field).changed() {
                    self.series = None;
                }
                if let Some(collection) = &self.series {
                    if collection.rows.is_empty() {
                        tokens::helper(ui, "No DICOM series found");
                    } else {
                        let label = |row: &dicom_browser::Series| {
                            let name = if row.cells[3].is_empty() {
                                &row.uid
                            } else {
                                &row.cells[3]
                            };
                            format!("{name}  {} × {} × {}", row.rows, row.cols, row.frames)
                        };
                        egui::ComboBox::from_label("Series")
                            .selected_text(label(&collection.rows[self.series_row]))
                            .show_ui(ui, |ui| {
                                for (index, row) in collection.rows.iter().enumerate() {
                                    ui.selectable_value(&mut self.series_row, index, label(row));
                                }
                            });
                    }
                }
            }
        }
    }

    fn main_caption(&self) -> &'static str {
        "3-D VIEW"
    }

    fn main_ui(&mut self, ui: &mut egui::Ui) {
        let size = egui::vec2(
            ui.available_width().max(1.0),
            ui.available_height().max(240.0),
        );
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::drag());
        self.note("3-D view", &response);
        ui.painter().rect_filled(
            rect,
            0.0,
            egui::Color32::from_rgb(
                self.view.background[0],
                self.view.background[1],
                self.view.background[2],
            ),
        );
        if let Some(texture) = &self.texture {
            let render = self.render.as_ref().unwrap();
            let [width, height] = render.viewport.size;
            let mut aspect = width as f64 / height as f64;
            if render.viewport.view == [0.0, 90.0]
                && let Some(data) = &self.loaded
            {
                let [rows, cols, _] = data.volume.dims;
                aspect = cols as f64 * render.settings.scale[0]
                    / (rows as f64 * render.settings.scale[1]);
            }
            paint_image(ui, rect, texture, aspect as f32);
        }
        if self.loaded.is_none() {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Import a volume or use sample data",
                egui::FontId::proportional(tokens::CAPTION),
                egui::Color32::WHITE,
            );
            return;
        }
        if response.dragged() {
            let delta = response.drag_delta();
            self.view.angles[0] =
                (self.view.angles[0] - 0.45 * f64::from(delta.x) + 180.0).rem_euclid(360.0) - 180.0;
            self.view.angles[1] =
                (self.view.angles[1] + 0.45 * f64::from(delta.y)).clamp(-90.0, 90.0);
        }
        if response.hovered() {
            let scroll = ui.input(|input| input.smooth_scroll_delta.y);
            if scroll != 0.0 {
                ui.input_mut(|input| input.smooth_scroll_delta.y = 0.0);
                self.view.zoom = (self.view.zoom * (f64::from(scroll) * 0.005).exp())
                    .clamp(*ZOOM_RANGE.start(), *ZOOM_RANGE.end());
            }
        }
    }

    fn results(&self) -> Vec<(&'static str, String)> {
        Vec::new()
    }

    fn names_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        super::name_field(ui, "Settings variable", &mut self.export_name, ws);
    }

    fn exported(&self, _: &Snapshot) -> Vec<String> {
        vec![self.export_name.clone()]
    }

    fn summary(&self) -> Option<String> {
        let data = self.loaded.as_ref()?;
        let [rows, cols, pages] = data.volume.dims;
        Some(format!("{rows} × {cols} × {pages}  {}", data.array.class()))
    }
}

fn pick_folder(_folder: Option<&Path>) -> Option<PathBuf> {
    #[cfg(not(test))]
    {
        let mut dialog = rfd::FileDialog::new();
        if let Some(folder) = _folder {
            dialog = dialog.set_directory(folder);
        }
        dialog.pick_folder()
    }
    #[cfg(test)]
    None
}

pub(crate) fn show(
    host: &mut Host<VolumeViewer>,
    ui: &mut egui::Ui,
    ws: &Snapshot,
    busy: bool,
    action: &mut Option<Action>,
) {
    refresh(host, ui.ctx());
    if !host.app.export_name_initialized {
        host.app.export_name = super::unused_name("renderingSettings", |name| ws.contains(name));
        host.app.export_name_initialized = true;
    }
    let mut sample = false;
    let mut scan = false;
    let mut load = false;
    let mut generate = false;
    let mut export = false;
    let mut load_ws_labels = None;
    let mut use_sample_labels = false;
    let rendered = host
        .app
        .current_render()
        .filter(|_| !busy && !host.saving && host.app.pending.is_none());
    let can_generate = rendered.is_some();
    let settings_name = rendered.map(|_| host.app.settings_name()).transpose();
    egui::Panel::bottom("volume_viewer_commands")
        .resizable(false)
        .frame(egui::Frame::new().inner_margin(tokens::S as i8))
        .show_pane(ui, |ui| {
            let error = host
                .run_error
                .as_deref()
                .or(host.app.import_error.as_deref())
                .or_else(|| settings_name.as_ref().err().map(String::as_str));
            if let Some(status) = host.app.status_text() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(status);
                });
            } else if let Some(error) = error {
                ui.label(egui::RichText::new(error).color(tokens::palette(ui.ctx()).error));
            } else {
                let summary = host.app.summary();
                let status = if host.message.is_empty() {
                    summary.as_deref().unwrap_or("No volume")
                } else {
                    &host.message
                };
                tokens::helper(ui, status);
            }
            ui.horizontal_wrapped(|ui| {
                let r = ui.menu_button("Import", |ui| {
                    let r = ui.button("Import from workspace");
                    host.app.note("Import from workspace", &r);
                    if r.clicked() {
                        host.app.import = Import::Workspace;
                        ui.close();
                    }
                    let r = ui.button("Import DICOM folder");
                    host.app.note("Import DICOM folder", &r);
                    if r.clicked() {
                        host.app.import = Import::Dicom;
                        host.app.browse_requested = true;
                        ui.close();
                    }
                });
                host.app.note("Import", &r.response);
                let r = ui.button("Use sample data");
                host.app.note("Use sample data", &r);
                sample = r.clicked();
                let r = ui.add_enabled(
                    matches!(settings_name, Ok(Some(_))),
                    egui::Button::new("Export rendering settings"),
                );
                host.app.note("Export rendering settings", &r);
                export = r.clicked();
                let r = ui.add_enabled(can_generate, egui::Button::new("Generate script"));
                host.app.note("Generate script", &r);
                generate = r.clicked();
            });
        });
    super::three_areas(
        ui,
        AppKind::VolumeViewer,
        host.app.main_caption(),
        |ui, area| match area {
            super::Area::Inputs => {
                host.app.inputs_ui(ui, ws);
                match host.app.import {
                    Import::None => {}
                    Import::Workspace => {
                        let ready = ws
                            .get(&host.app.workspace_name)
                            .is_some_and(|a| volume_shape(a).is_ok());
                        let r = ui.add_enabled(ready, egui::Button::new("Load workspace volume"));
                        host.app.note("Load workspace volume", &r);
                        load = r.clicked();
                    }
                    Import::Dicom => {
                        let r = ui.add_enabled(
                            !host.app.folder.trim().is_empty(),
                            egui::Button::new("Scan folder"),
                        );
                        host.app.note("Scan folder", &r);
                        scan = r.clicked();
                        let ready = host.app.series.as_ref().is_some_and(|c| !c.rows.is_empty());
                        let r = ui.add_enabled(ready, egui::Button::new("Load DICOM series"));
                        host.app.note("Load DICOM series", &r);
                        load = r.clicked();
                    }
                }
                host.app.rendering_controls(ui);
                let (ws_lbl, smp_lbl) = host.app.labels_ui(ui, ws);
                load_ws_labels = ws_lbl;
                use_sample_labels = smp_lbl;
                ui.add_space(tokens::M);
                host.app.names_ui(ui, ws);
            }
            super::Area::Main => host.app.main_ui(ui),
            super::Area::Output => host.app.slices_ui(ui),
        },
    );

    if ui.ctx().will_discard() {
        return;
    }
    if sample {
        host.app.import = Import::None;
        submit_sample(host, ui.ctx());
    }
    if std::mem::take(&mut host.app.browse_requested)
        && let Some(folder) = pick_folder(ws.folder())
    {
        host.app.folder = folder.to_string_lossy().into_owned();
        scan_folder(host, ui.ctx(), folder);
    }
    if scan {
        let folder = PathBuf::from(host.app.folder.trim());
        let folder = if folder.is_relative() {
            ws.folder()
                .map_or(folder.clone(), |base| base.join(&folder))
        } else {
            folder
        };
        scan_folder(host, ui.ctx(), folder);
    }
    if load {
        match host.app.import {
            Import::Workspace => {
                let name = host.app.workspace_name.clone();
                if let Some(array) = ws.get(&name) {
                    submit_array(host, ui.ctx(), array.clone(), name);
                }
            }
            Import::Dicom => {
                if let Err(error) = submit_series(host, ui.ctx()) {
                    host.app.import_error = Some(error);
                }
            }
            Import::None => {}
        }
    }
    if let Some((lname, larray)) = load_ws_labels
        && let Some(data) = &host.app.loaded
    {
        let dims = data.volume.dims;
        submit_labels(host, ui.ctx(), larray, lname, dims);
    }
    if use_sample_labels && let Some(data) = &host.app.loaded {
        let dims = data.volume.dims;
        submit_sample_labels(host, ui.ctx(), dims);
    }
    if export
        && let Ok(Some(name)) = settings_name
        && let Some(rendered) = host.app.current_render()
    {
        let script = format!("{name} = {};\n", rendered.settings.struct_expression());
        *action = Some(if let Some(labels) = &host.app.labels {
            Action::ExportVolumeSettings(Box::new(LabelSettingsExport {
                name,
                script,
                overlay: labels.array.clone(),
            }))
        } else {
            Action::ExportScript(script)
        });
    }
    if generate {
        match host.app.script(ws) {
            Ok(script) => {
                let code = super::saved_script(AppKind::VolumeViewer, super::now(), &script);
                let data = host.app.loaded.as_ref().unwrap();
                let companion_vol = (!data.sample).then(|| data.array.clone());
                let companion_lbl = host
                    .app
                    .labels
                    .as_ref()
                    .filter(|l| !l.sample)
                    .map(|l| (labels_script_var(l).to_owned(), l.array.clone()));
                *action = Some(if companion_vol.is_some() || companion_lbl.is_some() {
                    Action::SaveVolumeScript(code, companion_vol, companion_lbl)
                } else {
                    Action::SaveAppScript(AppKind::VolumeViewer, code)
                });
            }
            Err(error) => host.run_error = Some(error),
        }
    }
    submit_views(host, ui.ctx());
}

pub(crate) fn close(host: &mut Host<VolumeViewer>) {
    host.runner.stop();
    host.app = VolumeViewer::default();
    host.run_error = None;
    host.message.clear();
}

pub(crate) fn receive_open(workspace: &mut crate::Workspace) {
    let launch = workspace
        .worker
        .volume_viewer
        .lock()
        .ok()
        .and_then(|mut slot| slot.take());
    if let Some(launch) = launch
        && !workspace.worker.cancel.load(Ordering::Relaxed)
    {
        let tool = crate::toolkits::Tool::App(AppKind::VolumeViewer);
        if matches!(launch, Launch::Close) {
            workspace.close_tool_tab(tool);
        } else {
            workspace.toolkits.apps.volume_viewer.app.launch = Some(launch);
            workspace.open_app(tool);
        }
    }
}
