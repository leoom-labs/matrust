//! Colour ranges are MATLAB code; the engine owns conversion, masks and histograms.
use super::{App, AppKind, Host, Outputs, Snapshot, field, name_field, number};
use crate::theme::ThemedScrollArea;
use eframe::egui;
use matrust::Array;
use std::hash::{Hash, Hasher};

pub const DISPLAY_SIDE: usize = 2048;
const BINS: usize = 64;

/// Refuse an image whose element count is above `matrix::MAX_ELEMENTS`.
/// The message names the image size and that limit. `Ok` means the run may start.
pub(crate) fn reject_image_size(rows: usize, cols: usize, len: usize) -> Result<(), String> {
    let limit = matrust::matrix::MAX_ELEMENTS;
    if len > limit {
        return Err(format!(
            "Color Thresholder: {rows}-by-{cols}-by-3 image has {len} elements, above the limit of {limit}"
        ));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Space {
    Rgb,
    Hsv,
    Lab,
    Ycbcr,
}
impl Space {
    pub const ALL: [Self; 4] = [Self::Rgb, Self::Hsv, Self::Lab, Self::Ycbcr];
    pub fn label(self) -> &'static str {
        match self {
            Self::Rgb => "RGB",
            Self::Hsv => "HSV",
            Self::Lab => "L*a*b*",
            Self::Ycbcr => "YCbCr",
        }
    }
    fn channels(self) -> [&'static str; 3] {
        match self {
            Self::Rgb => ["Red", "Green", "Blue"],
            Self::Hsv => ["Hue", "Saturation", "Value"],
            Self::Lab => ["L*", "a*", "b*"],
            Self::Ycbcr => ["Y", "Cb", "Cr"],
        }
    }
    pub fn domains(self) -> [[f64; 2]; 3] {
        match self {
            Self::Lab => [[0., 100.], [-128., 127.], [-128., 127.]],
            _ => [[0., 1.]; 3],
        }
    }
    fn domain_guard(self) -> &'static str {
        if self == Self::Lab {
            "% L* is physically 0..100; remove matrix roundoff at white.\nI(:,:,1) = min(100,max(0,I(:,:,1)));\n"
        } else {
            ""
        }
    }
    fn conversion(self, source: &str) -> String {
        self.convert_normalized(&format!("im2double({source})"))
    }
    fn convert_normalized(self, source: &str) -> String {
        match self {
            Self::Rgb => source.into(),
            Self::Hsv => format!("rgb2hsv({source})"),
            Self::Lab => format!("rgb2lab({source})"),
            Self::Ycbcr => format!("rgb2ycbcr({source})"),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum View {
    Black,
    White,
    Mask,
}

struct Raster {
    key: u64,
    image: egui::ColorImage,
    texture: Option<(u64, egui::TextureHandle)>,
}
impl Raster {
    // Only bounded, engine-downsampled doubles reach the renderer. Column major
    // engine pixels become row major sRGB, just as truecolour figure images do.
    fn read(a: &Array) -> Result<Self, String> {
        let Array::Numeric(m) = a else {
            return Err("Color Thresholder: missing display image".into());
        };
        let dims = a.dims();
        let (rows, cols) = (dims[0], dims[1]);
        let channels = if dims.len() == 3 { dims[2] } else { 1 };
        if rows == 0
            || cols == 0
            || rows > DISPLAY_SIDE
            || cols > DISPLAY_SIDE
            || !matches!(channels, 1 | 3)
            || a.len() != rows * cols * channels
        {
            return Err("Color Thresholder: invalid display dimensions".into());
        }
        let mut rgb = Vec::with_capacity(rows * cols * 3);
        for y in 0..rows {
            for x in 0..cols {
                for c in 0..3 {
                    let at = y + x * rows + if channels == 3 { c * rows * cols } else { 0 };
                    rgb.push((m.data()[at].clamp(0., 1.) * 255.).round() as u8);
                }
            }
        }
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        (rows, cols).hash(&mut hash);
        rgb.hash(&mut hash);
        Ok(Self {
            key: hash.finish(),
            image: egui::ColorImage::from_rgb([cols, rows], &rgb),
            texture: None,
        })
    }
    fn draw(&mut self, ui: &mut egui::Ui, name: &str, size: egui::Vec2) {
        if self
            .texture
            .as_ref()
            .is_none_or(|(key, _)| *key != self.key)
        {
            self.texture = Some((
                self.key,
                ui.ctx()
                    .load_texture(name, self.image.clone(), egui::TextureOptions::NEAREST),
            ));
        }
        if let Some((_, texture)) = &self.texture {
            ui.add(
                egui::Image::new(texture)
                    .max_size(size)
                    .maintain_aspect_ratio(true),
            );
        }
    }
}

pub struct ResultView {
    images: [Raster; 3],
    pub selected: usize,
    pub total: usize,
    pub percentage: f64,
    pub histograms: [Vec<f64>; 3],
    pub limits: [[f64; 2]; 3],
    space: Space,
}

pub struct ColorThresholder {
    pub source: String,
    pub file: bool,
    pub space: Space,
    pub limits: [[String; 2]; 3],
    pub invert: bool,
    pub view: View,
    pub mask_name: String,
    pub image_name: String,
    pub current: Option<ResultView>,
    running: bool,
    stop: bool,
    resume: bool,
    paused: bool,
}
impl Default for ColorThresholder {
    fn default() -> Self {
        Self {
            source: String::new(),
            file: false,
            space: Space::Rgb,
            limits: std::array::from_fn(|_| ["0".into(), "1".into()]),
            invert: false,
            view: View::Black,
            mask_name: "BW".into(),
            image_name: "maskedRGBImage".into(),
            current: None,
            running: false,
            stop: false,
            resume: false,
            paused: false,
        }
    }
}

fn identifier(text: &str) -> Result<(), String> {
    if text.len() > 63 || !matrust::files::valid_name(text) {
        return Err("Use a variable name of at most 63 characters".into());
    }
    super::single_expression(text)?;
    matrust::syntax::parse(&format!("{text} = [];"))?;
    super::check_export_name(text)
}
fn scalar(text: &str) -> Result<f64, String> {
    if text.len() > 64 {
        return Err("Channel limit: at most 64 characters".into());
    }
    super::single_expression(text)?;
    text.trim()
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
        .ok_or_else(|| "Channel limit: enter one finite numeric literal".into())
}
fn image_shape(a: &Array) -> Result<(), String> {
    if !matches!(
        a.class(),
        "uint8" | "uint16" | "single" | "double" | "logical"
    ) || a.is_complex()
    {
        return Err("Color Thresholder: use uint8, uint16, single, double or logical RGB".into());
    }
    let dims = a.dims();
    if dims.len() != 3 || dims[2] != 3 || a.is_empty() {
        return Err(
            "Color Thresholder: an M-by-N-by-3 RGB image is required; gray input is not RGB".into(),
        );
    }
    // Before any private run: the engine never sees an image over its element cap.
    reject_image_size(dims[0], dims[1], a.len())
}

impl ColorThresholder {
    pub fn set_space(&mut self, space: Space) {
        self.space = space;
        self.limits = space.domains().map(|r| r.map(number));
    }
    pub fn ranges(&self) -> Result<[[f64; 2]; 3], String> {
        let mut ranges = [[0.; 2]; 3];
        for (c, range) in ranges.iter_mut().enumerate() {
            *range = [scalar(&self.limits[c][0])?, scalar(&self.limits[c][1])?];
            let domain = self.space.domains()[c];
            if range.iter().any(|v| !(domain[0]..=domain[1]).contains(v)) {
                return Err(format!(
                    "{} limits must be in [{}, {}]",
                    self.space.channels()[c],
                    domain[0],
                    domain[1]
                ));
            }
            if range[0] > range[1] && !(self.space == Space::Hsv && c == 0) {
                return Err("Only Hue may wrap: other minimums must not exceed maximums".into());
            }
        }
        Ok(ranges)
    }
    fn source_expression(&self, ws: &Snapshot) -> Result<String, String> {
        if self.file {
            if self.source.is_empty()
                || self.source.len() > 4096
                || self.source.contains(['\n', '\r', '\0'])
            {
                return Err("Choose an image file (path at most 4096 characters)".into());
            }
            let expr = format!("imread('{}')", self.source.replace('\'', "''"));
            super::single_expression(&expr)?;
            Ok(expr)
        } else {
            identifier(&self.source)?;
            image_shape(ws.get(&self.source).ok_or("Choose a workspace RGB image")?)?;
            Ok(self.source.clone())
        }
    }
    fn names(&self) -> Result<(), String> {
        identifier(&self.mask_name)?;
        identifier(&self.image_name)?;
        if self.mask_name == self.image_name {
            return Err("Mask and image export names must differ".into());
        }
        Ok(())
    }
    // One builder for the saved function, private preview and workspace export.
    // Only the function's identifier differs when a workspace name collides.
    pub fn function_text(&self, function: &str) -> Result<String, String> {
        let ranges = self.ranges()?;
        let limit = matrust::matrix::MAX_ELEMENTS;
        let mut text = format!(
            "function [BW, maskedRGBImage] = {function}(RGB)\n\
             % Colour limits apply to im2double RGB; Lab uses D65 sRGB.\n\
             if ~any(strcmp(class(RGB), {{'uint8', 'uint16', 'single', 'double', 'logical'}})) || ~isreal(RGB)\n\
                 error('Color Thresholder: unsupported RGB image class');\nend\n\
             if ndims(RGB) ~= 3 || size(RGB,3) ~= 3 || isempty(RGB)\n\
                 error('Color Thresholder: an M-by-N-by-3 RGB image is required; gray input is not RGB');\nend\n\
             if numel(RGB) > {limit}\n\
                 error('Color Thresholder: %d-by-%d-by-3 image has %d elements, above the limit of {limit}', size(RGB,1), size(RGB,2), numel(RGB));\nend\n\
             I = im2double(RGB);\n\
             for channel = 1:3\n\
                 plane = I(:,:,channel);\n\
                 if any(any(~isfinite(plane))) || any(any(plane < 0)) || any(any(plane > 1))\n\
                     error('Color Thresholder: floating RGB values must be finite and in [0,1]');\n\
                 end\n\
             end\n"
        );
        if self.space != Space::Rgb {
            text += &format!("I = {};\n", self.space.convert_normalized("I"));
        }
        text += self.space.domain_guard();
        for (c, [lo, hi]) in ranges.into_iter().enumerate() {
            text += &format!(
                "channel{}Min = {}; channel{}Max = {};\n",
                c + 1,
                number(lo),
                c + 1,
                number(hi)
            );
        }
        let parts: Vec<_> = (1..=3)
            .map(|c| {
                let op = if c == 1 && self.space == Space::Hsv && ranges[0][0] > ranges[0][1] {
                    "|"
                } else {
                    "&"
                };
                format!("(I(:,:,{c}) >= channel{c}Min {op} I(:,:,{c}) <= channel{c}Max)")
            })
            .collect();
        text += &format!("BW = {};\n", parts.join(" & "));
        if self.invert {
            text += "BW = ~BW;\n";
        }
        text += "clear I;\nmaskedRGBImage = RGB;\nbackground = 0;\n";
        if self.view == View::White {
            text += "background = 1;\nif strcmp(class(RGB), 'uint8'), background = 255; end\nif strcmp(class(RGB), 'uint16'), background = 65535; end\n";
        }
        text += "for channel = 1:3\n    plane = RGB(:,:,channel);\n    plane(~BW) = background;\n    maskedRGBImage(:,:,channel) = plane;\nend\nend\n";
        Ok(text)
    }
    fn private_names(&self, ws: &Snapshot) -> [String; 3] {
        ["ct_source", "ct_apply", "ct_view"].map(|base| {
            super::unused_name(base, |n| {
                ws.contains(n) || n == self.mask_name || n == self.image_name
            })
        })
    }
}

impl App for ColorThresholder {
    fn validate_idle_inputs(&self, _ws: &Snapshot) -> Result<(), String> {
        self.names()
    }
    fn idle_hint(&self) -> Option<&'static str> {
        self.source.trim().is_empty().then_some(if self.file {
            "Choose an image file"
        } else {
            "Choose a workspace RGB image"
        })
    }
    fn kind(&self) -> AppKind {
        AppKind::ColorThresholder
    }
    fn script(&self, ws: &Snapshot) -> Result<String, String> {
        self.source_expression(ws)?;
        self.names()?;
        self.function_text("createMask")
    }
    fn execution_script(&self, ws: &Snapshot) -> Result<String, String> {
        if self.paused {
            return Err("Color Thresholder: stopped; choose Recompute / reload to continue".into());
        }
        self.names()?;
        let expr = self.source_expression(ws)?;
        let [source, function, _] = self.private_names(ws);
        Ok(format!(
            "{source} = {expr};\n[{}, {}] = {function}({source});\n{}",
            self.mask_name,
            self.image_name,
            self.function_text(&function)?
        ))
    }
    fn inputs(&self) -> Vec<String> {
        if self.file {
            Vec::new()
        } else {
            vec![self.source.clone()]
        }
    }
    fn probe(&self, ws: &Snapshot) -> (String, Vec<String>) {
        let [source, _, function] = self.private_names(ws);
        let outputs: Vec<String> = [
            "ct_original",
            "ct_masked",
            "ct_mask",
            "ct_counts",
            "ct_hist",
            "ct_limits",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        let mut text = format!(
            "[{}] = {function}({source}, {}, {});\nfunction [original, masked, mask, counts, histogram, limits] = {function}(RGB, BW, maskedRGBImage)\n",
            outputs.join(", "),
            self.mask_name,
            self.image_name
        );
        text += &format!(
            "stride = max(1,ceil(max(size(RGB,1),size(RGB,2))/{DISPLAY_SIDE}));\n\
            original = im2double(RGB(1:stride:end,1:stride:end,:));\n\
            masked = im2double(maskedRGBImage(1:stride:end,1:stride:end,:));\n\
            mask = double(BW(1:stride:end,1:stride:end));\n\
            counts = [nnz(BW),numel(BW)]; counts(3) = 100*counts(1)/counts(2);\n\
            I = {};\nhistogram = zeros({BINS},3);\n",
            self.space.conversion("RGB")
        );
        text += self.space.domain_guard();
        for (c, [lo, hi]) in self.space.domains().into_iter().enumerate() {
            text += &format!(
                "plane = I(:,:,{});\nhistogram(:,{}) = histcounts(plane,linspace({},{},{}))';\n",
                c + 1,
                c + 1,
                number(lo),
                number(hi),
                BINS + 1
            );
        }
        let ranges = self.ranges().unwrap_or([[0., 1.]; 3]);
        text += &format!(
            "limits = [{}; {}; {}];\nend\n",
            ranges[0].map(number).join(" "),
            ranges[1].map(number).join(" "),
            ranges[2].map(number).join(" ")
        );
        (text, outputs)
    }
    fn receive(&mut self, outputs: &Outputs) -> Result<(), String> {
        let read = |name: &str| outputs.get(name).ok_or_else(|| format!("Missing {name}"));
        let numeric = |name: &str| -> Result<&[f64], String> {
            match read(name)? {
                Array::Numeric(m) => Ok(m.data()),
                _ => Err(format!("Invalid {name}")),
            }
        };
        let mut images = [
            Raster::read(read("ct_original")?)?,
            Raster::read(read("ct_masked")?)?,
            Raster::read(read("ct_mask")?)?,
        ];
        let counts = numeric("ct_counts")?;
        let hist = numeric("ct_hist")?;
        let limits = numeric("ct_limits")?;
        if counts.len() != 3 || hist.len() != 3 * BINS || limits.len() != 6 {
            return Err("Color Thresholder: incomplete result".into());
        }
        if let Some(old) = &self.current {
            for (new, old) in images.iter_mut().zip(&old.images) {
                if new.key == old.key {
                    new.texture = old.texture.clone();
                }
            }
        }
        self.current = Some(ResultView {
            images,
            selected: counts[0] as usize,
            total: counts[1] as usize,
            percentage: counts[2],
            histograms: std::array::from_fn(|c| hist[c * BINS..(c + 1) * BINS].to_vec()),
            limits: std::array::from_fn(|c| [limits[c], limits[c + 3]]),
            space: self.space,
        });
        Ok(())
    }
    fn inputs_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        field(ui, "Source", |ui| {
            if ui.selectable_label(!self.file, "Workspace").clicked() {
                self.file = false;
                self.source.clear();
            }
            if ui.selectable_label(self.file, "File").clicked() {
                self.file = true;
                self.source.clear();
            }
        });
        if self.file {
            field(ui, "Image path (imread)", |ui| {
                ui.add(crate::tokens::input(egui::TextEdit::singleline(
                    &mut self.source,
                )));
            });
            if ui.button("Choose image…").clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("Images", &["png", "jpg", "jpeg", "tif", "tiff", "bmp"])
                    .pick_file()
            {
                self.source = path.to_string_lossy().into_owned();
            }
        } else {
            field(ui, "RGB image variable", |ui| {
                egui::ComboBox::from_id_salt("ct_source")
                    .selected_text(if self.source.is_empty() {
                        "Choose image"
                    } else {
                        &self.source
                    })
                    .show_ui(ui, |ui| {
                        for (name, a) in ws.variables().filter(|(_, a)| {
                            matches!(
                                a.class(),
                                "double" | "single" | "uint8" | "uint16" | "logical"
                            )
                        }) {
                            ui.selectable_value(
                                &mut self.source,
                                name.to_owned(),
                                format!("{name} ({})", a.class()),
                            );
                        }
                    });
                ui.add(crate::tokens::input(egui::TextEdit::singleline(
                    &mut self.source,
                )));
            });
        }
        let mut space = self.space;
        field(ui, "Colour space", |ui| {
            egui::ComboBox::from_id_salt("ct_space")
                .selected_text(space.label())
                .show_ui(ui, |ui| {
                    for s in Space::ALL {
                        ui.selectable_value(&mut space, s, s.label());
                    }
                });
        });
        if space != self.space {
            self.set_space(space);
        }
        ui.checkbox(&mut self.invert, "Invert mask");
        crate::tokens::helper(
            ui,
            "RGB, HSV and YCbCr limits use 0–1. Hue minimum above maximum selects across zero.",
        );
        if self.running && ui.button("Stop").clicked() {
            self.stop = true;
        }
        if !self.running && ui.button("Recompute / reload").clicked() {
            self.resume = true;
        }
    }
    fn main_caption(&self) -> &'static str {
        "IMAGE AND COLOUR RANGES"
    }
    fn main_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            for (view, label) in [
                (View::Black, "Black background"),
                (View::White, "White background"),
                (View::Mask, "Mask"),
            ] {
                if crate::tokens::toggle(ui, self.view == view, label).clicked() {
                    self.view = view;
                }
            }
        });
        if let Some(result) = &mut self.current {
            let size = egui::vec2((ui.available_width() - crate::tokens::S) / 2., 280.);
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label("Source");
                    result.images[0].draw(ui, "ct_source_image", size);
                });
                ui.vertical(|ui| {
                    ui.label("Selection");
                    result.images[if self.view == View::Mask { 2 } else { 1 }].draw(
                        ui,
                        "ct_selection_image",
                        size,
                    );
                });
            });
        } else {
            crate::tokens::helper(ui, "Choose an RGB image to see the source and selection.");
        }
        for c in 0..3 {
            ui.add_space(crate::tokens::S);
            ui.label(self.space.channels()[c]);
            let bins = self
                .current
                .as_ref()
                .filter(|r| r.space == self.space)
                .map(|r| r.histograms[c].as_slice())
                .unwrap_or(&[]);
            ui.add_enabled_ui(
                self.current.is_some() && !self.source.trim().is_empty(),
                |ui| {
                    range_histogram(ui, c, bins, self.space.domains()[c], &mut self.limits[c]);
                },
            );
        }
        egui::CollapsingHeader::new("Function preview").show(ui, |ui| {
            if let Ok(code) = self.function_text("createMask") {
                egui::ScrollArea::vertical()
                    .max_height(240.)
                    .show_themed(ui, |ui| {
                        // Same proportional typeface as the rest of the app.
                        ui.add(egui::Label::new(code).selectable(true));
                    });
            }
        });
    }
    fn results(&self) -> Vec<(&'static str, String)> {
        let Some(result) = &self.current else {
            return Vec::new();
        };
        let mut rows = vec![
            (
                "Selected pixels",
                format!("{} / {}", result.selected, result.total),
            ),
            ("Selected", format!("{}%", super::short(result.percentage))),
        ];
        for (c, label) in ["Channel 1 limits", "Channel 2 limits", "Channel 3 limits"]
            .into_iter()
            .enumerate()
        {
            rows.push((
                label,
                format!(
                    "{} … {}",
                    number(result.limits[c][0]),
                    number(result.limits[c][1])
                ),
            ));
        }
        rows
    }
    fn names_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        name_field(ui, "Logical mask", &mut self.mask_name, ws);
        name_field(ui, "Masked RGB image", &mut self.image_name, ws);
    }
    fn exported(&self, _: &Snapshot) -> Vec<String> {
        vec![self.mask_name.clone(), self.image_name.clone()]
    }
    fn summary(&self) -> Option<String> {
        self.current.as_ref().map(|r| {
            format!(
                "Color Thresholder: {} of {} pixels selected",
                r.selected, r.total
            )
        })
    }
}

// Both handles share the histogram's coordinate system; exact literals remain
// independently editable, including crossed handles for circular Hue.
fn range_histogram(
    ui: &mut egui::Ui,
    channel: usize,
    bins: &[f64],
    domain: [f64; 2],
    limits: &mut [String; 2],
) {
    ui.horizontal(|ui| {
        let palette = crate::tokens::palette(ui.ctx());
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(
                (ui.available_width() - 184. - crate::tokens::S).max(64.),
                72.,
            ),
            egui::Sense::click_and_drag(),
        );
        let mut values = [
            scalar(&limits[0]).unwrap_or(domain[0]),
            scalar(&limits[1]).unwrap_or(domain[1]),
        ];
        let x = |v: f64| {
            rect.left() + ((v - domain[0]) / (domain[1] - domain[0])) as f32 * rect.width()
        };
        ui.painter().hline(
            rect.x_range(),
            rect.center().y,
            egui::Stroke::new(1., palette.border),
        );
        let peak = bins.iter().copied().fold(1., f64::max);
        for (i, v) in bins.iter().enumerate() {
            let left = rect.left() + i as f32 * rect.width() / BINS as f32;
            let right = rect.left() + (i + 1) as f32 * rect.width() / BINS as f32;
            ui.painter().rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(left, rect.bottom() - (*v / peak) as f32 * 64.),
                    egui::pos2(right, rect.bottom()),
                ),
                0.,
                palette.text_muted,
            );
        }
        let handle_id = ui.id().with(("ct_handle", channel));
        if (response.drag_started() || response.clicked())
            && let Some(pos) = response.interact_pointer_pos()
        {
            let chosen = usize::from((pos.x - x(values[1])).abs() < (pos.x - x(values[0])).abs());
            ui.ctx()
                .data_mut(|data| data.insert_temp(handle_id, chosen));
        }
        if (response.dragged() || response.clicked())
            && let Some(pos) = response.interact_pointer_pos()
        {
            let chosen = ui
                .ctx()
                .data(|data| data.get_temp::<usize>(handle_id))
                .unwrap_or(0);
            values[chosen] = domain[0]
                + ((pos.x - rect.left()) / rect.width()).clamp(0., 1.) as f64
                    * (domain[1] - domain[0]);
            limits[chosen] = number(values[chosen]);
        }
        for v in values {
            let at = x(v.clamp(domain[0], domain[1]));
            ui.painter().line_segment(
                [egui::pos2(at, rect.top()), egui::pos2(at, rect.bottom())],
                egui::Stroke::new(2., palette.accent),
            );
            ui.painter()
                .circle_filled(egui::pos2(at, rect.top() + 4.), 4., palette.accent);
        }
        ui.vertical(|ui| {
            for (i, label) in ["Minimum", "Maximum"].into_iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(label);
                    ui.add(crate::tokens::input(
                        egui::TextEdit::singleline(&mut limits[i]).desired_width(88.),
                    ));
                });
            }
        });
    });
}

pub(crate) fn show(
    host: &mut Host<ColorThresholder>,
    ui: &mut egui::Ui,
    ws: &Snapshot,
    busy: bool,
    action: &mut Option<crate::toolkits::Action>,
) {
    host.set_folder(ws.folder());
    host.app.running = host.running();
    host.show(ui, ws, busy, action);
    if std::mem::take(&mut host.app.stop) {
        host.runner.stop();
        host.app.paused = true;
        host.message = "Stopped; the last successful result is retained".into();
    }
    if std::mem::take(&mut host.app.resume) {
        host.app.paused = false;
        host.runner.stop();
        host.delivered = None;
        host.message.clear();
    }
}
