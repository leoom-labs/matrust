//! Image inspection. One script builds every export; private probes only read
//! its full-resolution arrays. The Runner owns all image operations, including
//! display reduction. Rust draws a cached texture and reads individual pixels.
use super::{App, AppKind, Host, Outputs, Snapshot, field, name_field, number, short, unused_name};
use eframe::egui;
use matrust::Array;
use matrust::typed_array::TypedData;
use std::collections::BTreeSet;
use std::hash::{Hash, Hasher};

pub const MAX_DISTANCES: usize = 128;
pub const DISPLAY_SIDE: usize = 2048;

/// Refuse an image whose element count is above `matrix::MAX_ELEMENTS`.
/// The message names the image size and that limit. `Ok` means the run may start.
pub(crate) fn reject_image_size(
    rows: usize,
    cols: usize,
    channels: usize,
    len: usize,
) -> Result<(), String> {
    let limit = matrust::matrix::MAX_ELEMENTS;
    if len > limit {
        return Err(format!(
            "Image Viewer: {rows}-by-{cols}-by-{channels} image has {len} elements, above the limit of {limit}"
        ));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tool {
    #[default]
    Pan,
    Crop,
    Distance,
    Contrast,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Contrast {
    #[default]
    Original,
    Window,
    DataRange,
    Saturate,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Crop {
    pub x: usize,
    pub y: usize,
    /// imcrop uses the distance between pixel centres: output = width+1.
    pub width: usize,
    pub height: usize,
}
impl Crop {
    pub fn from_points(a: [usize; 2], b: [usize; 2]) -> Self {
        Self {
            x: a[0].min(b[0]),
            y: a[1].min(b[1]),
            width: a[0].abs_diff(b[0]),
            height: a[1].abs_diff(b[1]),
        }
    }
    fn vector(self) -> String {
        format!("[{} {} {} {}]", self.x, self.y, self.width, self.height)
    }
}

pub struct Shown {
    pub image: Array,
    pub map: Array,
    pub display: Array,
    pub histogram: Vec<Vec<f64>>,
    pub limits: Vec<f64>,
    /// [x1 y1 x2 y2 length_pixels angle_degrees], one row per distance.
    pub distances: Vec<[f64; 6]>,
    pub source_size: [usize; 2],
    pub crop: Option<Crop>,
    pub file_info: String,
    pub key: u64,
}

pub struct ImageViewer {
    pub file_source: bool,
    pub source: String,
    pub path: String,
    pub colormap: String,
    pub image_name: String,
    pub distance_name: String,
    pub map_name: String,
    pub replace: bool,
    pub crop: Option<Crop>,
    pub contrast: Contrast,
    pub window: [f64; 2],
    pub lines: Vec<[[f64; 2]; 2]>,
    pub tool: Tool,
    pub current: Option<Shown>,
    pub zoom: Option<f32>,
    pub pan: egui::Vec2,
    texture: Option<(u64, egui::TextureHandle)>,
    drag_start: Option<[usize; 2]>,
    drag_end: Option<[usize; 2]>,
    running: bool,
    stopped: bool,
    stop_requested: bool,
}
impl Default for ImageViewer {
    fn default() -> Self {
        Self {
            file_source: false,
            source: String::new(),
            path: String::new(),
            colormap: String::new(),
            image_name: "viewed_image".into(),
            distance_name: "image_distances".into(),
            map_name: "viewed_map".into(),
            replace: false,
            crop: None,
            contrast: Contrast::Original,
            window: [0., 1.],
            lines: Vec::new(),
            tool: Tool::Pan,
            current: None,
            zoom: None,
            pan: egui::Vec2::ZERO,
            texture: None,
            drag_start: None,
            drag_end: None,
            running: false,
            stopped: false,
            stop_requested: false,
        }
    }
}
struct Plan {
    script: String,
    image: String,
    map: String,
    source: String,
    limits: String,
    distances: String,
    info: String,
}
fn identifier(text: &str) -> Result<(), String> {
    if text.len() > 63 {
        return Err("Use a variable name of at most 63 characters".into());
    }
    super::single_expression(text)?;
    if !matrust::files::valid_name(text) {
        return Err("Choose one workspace variable name".into());
    }
    Ok(())
}
pub fn image_shape(a: &Array) -> Result<[usize; 3], String> {
    let dims = a.dims();
    let channels = dims.get(2).copied().unwrap_or(1);
    if !matches!(a, Array::Numeric(_) | Array::Logical(_) | Array::Typed(_))
        || !matches!(
            a.class(),
            "uint8" | "uint16" | "double" | "single" | "logical"
        )
        || a.is_complex()
        || a.is_empty()
        || dims.len() > 3
        || !matches!(channels, 1 | 3)
    {
        return Err(
            "Choose a real gray or RGB uint8, uint16, single, double or logical image".into(),
        );
    }
    // Before any private run: the engine never sees an image over its element cap.
    reject_image_size(dims[0], dims[1], channels, a.len())?;
    Ok([dims[0], dims[1], channels])
}
fn sample(a: &Array, i: usize) -> f64 {
    match a {
        Array::Numeric(m) => m.data()[i],
        Array::Logical(m) => f64::from(m.data()[i]),
        Array::Typed(m) => m.real(i),
        _ => unreachable!("validated image"),
    }
}
/// 1-based displayed row and column; the class's original units are preserved.
pub fn pixel(a: &Array, map: &Array, row: usize, col: usize) -> Option<String> {
    let [h, w, channels] = image_shape(a).ok()?;
    if row == 0 || col == 0 || row > h || col > w {
        return None;
    }
    let at = row - 1 + (col - 1) * h;
    let values: Vec<_> = (0..channels).map(|c| sample(a, at + c * h * w)).collect();
    let value = values
        .iter()
        .map(|v| number(*v))
        .collect::<Vec<_>>()
        .join(", ");
    let mut text = format!("Row {row} · column {col} · {} [{value}]", a.class());
    if !map.is_empty() {
        let zero_based = matches!(a.class(), "uint8" | "uint16");
        let index = (values[0] - if zero_based { 0. } else { 1. })
            .floor()
            .clamp(0., (map.rows() - 1) as f64) as usize;
        let rgb = (0..3)
            .map(|c| number(sample(map, index + c * map.rows())))
            .collect::<Vec<_>>()
            .join(", ");
        text.push_str(&format!(" · index {value} · RGB [{rgb}]"));
    }
    Some(text)
}
impl ImageViewer {
    fn checked_settings(&self, ws: &Snapshot) -> Result<BTreeSet<String>, String> {
        if self.stopped {
            return Err("Image Viewer stopped; press Resume to compute".into());
        }
        let mut used = BTreeSet::new();
        for name in [&self.image_name, &self.distance_name, &self.map_name] {
            identifier(name)?;
            super::check_export_name(name)?;
            if matches!(
                name.as_str(),
                "size" | "im2uint8" | "imhist" | "ind2rgb" | "zeros" | "ceil" | "max" | "isempty"
            ) {
                return Err(format!(
                    "{name} is reserved by Image Viewer; choose another export name"
                ));
            }
            if !used.insert(name.clone()) {
                return Err("Image Viewer: export names must be distinct".into());
            }
            if ws.contains(name) && !self.replace {
                return Err(format!(
                    "{name} exists; choose another name or enable Replace existing variables"
                ));
            }
        }
        if self.lines.len() > MAX_DISTANCES {
            return Err(format!("At most {MAX_DISTANCES} distances"));
        }
        let limit = matrust::matrix::MAX_ELEMENTS;
        for line in &self.lines {
            if line
                .iter()
                .flatten()
                .any(|v| !v.is_finite() || *v < 1. || *v > limit as f64)
            {
                return Err("Distance endpoints must be finite image coordinates".into());
            }
        }
        if self.contrast == Contrast::Window
            && (!self.window.iter().all(|v| v.is_finite())
                || self.window[0] < 0.
                || self.window[1] > 1.
                || self.window[0] >= self.window[1])
        {
            return Err("Contrast needs 0 ≤ minimum < maximum ≤ 1".into());
        }
        if let Some(c) = self.crop
            && (c.x == 0
                || c.y == 0
                || c.x > limit
                || c.y > limit
                || c.width >= limit
                || c.height >= limit)
        {
            return Err(
                "Crop requires positive integer x/y and nonnegative integer width/height".into(),
            );
        }
        if !self.colormap.trim().is_empty() {
            identifier(&self.colormap)?;
            let a = ws
                .get(&self.colormap)
                .ok_or("Choose a workspace colormap")?;
            if !matches!(a, Array::Numeric(_) | Array::Typed(_))
                || a.is_complex()
                || a.dims().len() != 2
                || a.cols() != 3
                || a.rows() == 0
                || a.rows() > 65536
            {
                return Err("Colormap must be a real N-by-3 numeric array (1–65536 colors)".into());
            }
        }
        Ok(used)
    }
    fn plan(&self, ws: &Snapshot) -> Result<Plan, String> {
        let mut used = self.checked_settings(ws)?;
        let limit = matrust::matrix::MAX_ELEMENTS;
        let mut fresh = |base: &str| {
            let name = unused_name(base, |n| ws.contains(n) || used.contains(n));
            used.insert(name.clone());
            name
        };
        let source = fresh("iv_source");
        let image = fresh("iv_image");
        let map = fresh("iv_map");
        let info = fresh("iv_info");
        let limits = fresh("iv_limits");
        let distances = fresh("iv_distances");
        let contrast_source = fresh("iv_contrast_source");
        let mut script = String::from(
            "% Image Viewer: full-resolution pixels; coordinates are [column row].\n% Crop [x y width height] includes both endpoint pixel centres.\n% Distances: [x1 y1 x2 y2 length_pixels angle_degrees]; positive angle points down.\n",
        );
        if self.file_source {
            if self.path.is_empty()
                || self.path.len() > 4096
                || self.path.contains(['\n', '\r', '\0'])
            {
                return Err("Choose an image file (path at most 4096 characters)".into());
            }
            let literal = format!("'{}'", self.path.replace('\'', "''"));
            super::single_expression(&literal)?;
            script.push_str(&format!(
                "{info} = imfinfo({literal});\n\
                 if strcmp({info}.ColorType,'truecolor')\n\
                     if {info}.Width*{info}.Height*3 > {limit}, error('Image Viewer: %d-by-%d-by-3 image has %d elements, above the limit of {limit}', {info}.Height, {info}.Width, {info}.Width*{info}.Height*3); end\n\
                 elseif {info}.Width*{info}.Height > {limit}\n\
                     error('Image Viewer: %d-by-%d-by-1 image has %d elements, above the limit of {limit}', {info}.Height, {info}.Width, {info}.Width*{info}.Height);\n\
                 end\n\
                 [{source}, {map}] = imread({literal});\n"
            ));
        } else {
            identifier(&self.source)?;
            image_shape(ws.get(&self.source).ok_or("Choose a workspace image")?)?;
            script.push_str(&format!(
                "{source} = {};\n{map} = [];\n{info} = [];\n",
                self.source
            ));
        }
        if !self.colormap.trim().is_empty() {
            script.push_str(&format!("{map} = double({});\n", self.colormap));
        }
        script.push_str(&format!(
            "if isempty({source}) || ndims({source}) > 3 || (size({source},3) ~= 1 && size({source},3) ~= 3) || ~isreal({source}), error('Image Viewer: expected a real gray or RGB image'); end\n\
             if ~any(strcmp(class({source}), {{'uint8', 'uint16', 'single', 'double', 'logical'}})), error('Image Viewer: unsupported image class'); end\n\
             if numel({source}) > {limit}, error('Image Viewer: %d-by-%d-by-%d image has %d elements, above the limit of {limit}', size({source},1), size({source},2), size({source},3), numel({source})); end\n\
             if ~isempty({map}) && (size({map},2) ~= 3 || size({source},3) ~= 1 || size({map},1) > 65536 || any(~isfinite({map}(:))) || any({map}(:) < 0) || any({map}(:) > 1)), error('Image Viewer: invalid indexed image colormap'); end\n"
        ));
        if let Some(c) = self.crop {
            script.push_str(&format!("if {} > size({source},2) || {} > size({source},1), error('Image Viewer: crop exceeds image bounds'); end\n{image} = imcrop({source}, {});\n", c.x + c.width, c.y + c.height, c.vector()));
        } else {
            script.push_str(&format!("{image} = {source};\n"));
        }
        script.push_str(&format!("{limits} = [0; 1];\n"));
        if self.contrast != Contrast::Original {
            script.push_str(&format!("{contrast_source} = {image};\nif ~isempty({map}), {contrast_source} = ind2rgb({image}, {map}); end\n"));
            let expr = match self.contrast {
                Contrast::DataRange => format!("stretchlim({contrast_source}, 0)"),
                Contrast::Saturate => format!("stretchlim({contrast_source}, 0.01)"),
                Contrast::Window => {
                    format!("[{}; {}]", number(self.window[0]), number(self.window[1]))
                }
                Contrast::Original => unreachable!(),
            };
            script.push_str(&format!("{limits} = {expr};\nif isempty({map}), if strcmp(class({image}),'logical'), {image} = logical(imadjust(im2double({image}), {limits}, [0; 1])); else, {image} = imadjust({image}, {limits}, [0; 1]); end; else, {map} = reshape(imadjust(reshape({map},[size({map},1) 1 3]), {limits}, [0; 1]),[size({map},1) 3]); end\n"));
        }
        script.push_str(&format!("{distances} = zeros(0,6);\n"));
        for (i, [a, b]) in self.lines.iter().enumerate() {
            let [x1, y1, x2, y2] = [a[0], a[1], b[0], b[1]].map(number);
            script.push_str(&format!("if max([{x1} {x2}]) > size({source},2) || max([{y1} {y2}]) > size({source},1), error('Image Viewer: distance endpoint outside image'); end\n{distances}({},:) = [{x1} {y1} {x2} {y2} hypot({x2}-{x1},{y2}-{y1}) atan2d({y2}-{y1},{x2}-{x1})];\n", i+1));
        }
        // All reads and function calls precede public assignments; names may
        // replace sources or builtins only after their final use in the script.
        script.push_str(&format!(
            "{} = {image};\n{} = {distances};\n{} = {map};\n",
            self.image_name, self.distance_name, self.map_name
        ));
        Ok(Plan {
            script,
            image,
            map,
            source,
            limits,
            distances,
            info,
        })
    }
}

impl App for ImageViewer {
    fn validate_idle_inputs(&self, ws: &Snapshot) -> Result<(), String> {
        self.checked_settings(ws).map(|_| ())
    }
    fn idle_hint(&self) -> Option<&'static str> {
        if self.file_source {
            self.path
                .trim()
                .is_empty()
                .then_some("Choose an image file")
        } else {
            self.source
                .trim()
                .is_empty()
                .then_some("Choose a workspace image")
        }
    }
    fn kind(&self) -> AppKind {
        AppKind::ImageViewer
    }
    fn script(&self, ws: &Snapshot) -> Result<String, String> {
        Ok(self.plan(ws)?.script)
    }
    fn inputs(&self) -> Vec<String> {
        let mut names = BTreeSet::new();
        if !self.file_source {
            names.insert(self.source.clone());
        }
        if !self.colormap.trim().is_empty() {
            names.insert(self.colormap.clone());
        }
        names.into_iter().collect()
    }
    fn probe(&self, ws: &Snapshot) -> (String, Vec<String>) {
        let Ok(p) = self.plan(ws) else {
            return (String::new(), Vec::new());
        };
        let (a, m, s, l, d, f) = (
            &p.image,
            &p.map,
            &p.source,
            &p.limits,
            &p.distances,
            &p.info,
        );
        let code = format!(
            "iv_probe_image = {a};\niv_probe_map = {m};\niv_probe_limits = {l};\niv_probe_distances = {d};\niv_probe_size = [size({s},1) size({s},2)];\niv_probe_meta = []; iv_probe_color = '';\nif ~isempty({f}), iv_probe_meta = [{f}.Width {f}.Height {f}.BitDepth {f}.FileSize]; iv_probe_color = {f}.ColorType; end\niv_probe_step = max(1,ceil(max(size({a},1),size({a},2))/{DISPLAY_SIDE}));\niv_probe_small = {a}(1:iv_probe_step:end,1:iv_probe_step:end,:);\niv_probe_hist_image = {a};\nif ~isempty({m}), iv_probe_small = ind2rgb(iv_probe_small,{m}); iv_probe_hist_image = ind2rgb({a},{m}); end\niv_probe_display = im2uint8(iv_probe_small);\niv_probe_hist = zeros(256,size(iv_probe_hist_image,3));\nfor iv_probe_channel=1:size(iv_probe_hist_image,3), iv_probe_hist(:,iv_probe_channel) = imhist(iv_probe_hist_image(:,:,iv_probe_channel),256); end\n"
        );
        (
            code,
            [
                "image",
                "map",
                "limits",
                "distances",
                "size",
                "meta",
                "color",
                "display",
                "hist",
            ]
            .map(|n| format!("iv_probe_{n}"))
            .to_vec(),
        )
    }
    fn receive(&mut self, out: &Outputs) -> Result<(), String> {
        let read = |part: &str| {
            out.get(&format!("iv_probe_{part}"))
                .cloned()
                .ok_or_else(|| format!("Image Viewer: missing {part}"))
        };
        let numeric = |part: &str| -> Result<Vec<f64>, String> {
            match read(part)? {
                Array::Numeric(a) => Ok(a.data().to_vec()),
                _ => Err(format!("Image Viewer: invalid {part}")),
            }
        };
        let image = read("image")?;
        image_shape(&image)?;
        let display = read("display")?;
        let [h, w, _] = image_shape(&display)?;
        let Array::Typed(bytes) = &display else {
            return Err("Image Viewer: expected uint8 display".into());
        };
        let TypedData::Uint8(bytes) = bytes.data() else {
            return Err("Image Viewer: expected uint8 display".into());
        };
        if h > DISPLAY_SIDE || w > DISPLAY_SIDE {
            return Err("Image Viewer: oversized display".into());
        }
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        (h, w, display.dims().get(2).copied().unwrap_or(1)).hash(&mut hash);
        // Hash only the bounded display raster, once per delivered run. A
        // distance or output-name edit then reuses the unchanged texture.
        bytes.as_slice().hash(&mut hash);
        let hist = numeric("hist")?;
        if !matches!(hist.len(), 256 | 768) {
            return Err("Image Viewer: incomplete histogram".into());
        }
        let distance_data = numeric("distances")?;
        if !distance_data.len().is_multiple_of(6) || distance_data.len() > 6 * MAX_DISTANCES {
            return Err("Image Viewer: incomplete distances".into());
        }
        let limits = numeric("limits")?;
        if !matches!(limits.len(), 2 | 6) || limits.iter().any(|v| !v.is_finite()) {
            return Err("Image Viewer: incomplete contrast limits".into());
        }
        let n = distance_data.len() / 6;
        let distances = (0..n)
            .map(|i| std::array::from_fn(|c| distance_data[i + c * n]))
            .collect();
        let size = numeric("size")?;
        if size.len() != 2 {
            return Err("Image Viewer: missing source size".into());
        }
        let meta = numeric("meta")?;
        let color = match read("color")? {
            Array::Character(c) => String::from_utf16_lossy(c.units()),
            _ => String::new(),
        };
        let file_info = if meta.len() == 4 {
            format!(
                "{} × {} · {} · {} bit · {color} · {} bytes",
                meta[0],
                meta[1],
                image.class(),
                meta[2],
                meta[3]
            )
        } else {
            String::new()
        };
        let shown = Shown {
            image,
            map: read("map")?,
            display,
            histogram: hist.chunks(256).map(<[f64]>::to_vec).collect(),
            limits,
            distances,
            source_size: [size[0] as usize, size[1] as usize],
            crop: self.crop,
            file_info,
            key: hash.finish(),
        };
        if matches!(self.contrast, Contrast::DataRange | Contrast::Saturate) {
            self.window = [shown.limits[0], shown.limits[1]];
        }
        self.current = Some(shown);
        Ok(())
    }
    fn inputs_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        ui.horizontal(|ui| {
            let response = ui.selectable_value(&mut self.file_source, false, "Variable");
            crate::focus_areas::register(
                ui,
                &response,
                crate::focus_areas::Area::App(AppKind::ImageViewer),
            );
            ui.selectable_value(&mut self.file_source, true, "File");
        });
        if self.file_source {
            text_field(ui, "Image file", &mut self.path, 4096);
        } else {
            field(ui, "Workspace image", |ui| {
                egui::ComboBox::from_id_salt("iv_source_picker")
                    .width(ui.available_width())
                    .selected_text(if self.source.is_empty() {
                        "Choose image"
                    } else {
                        &self.source
                    })
                    .show_ui(ui, |ui| {
                        for (name, _) in ws.variables().filter(|(_, a)| image_shape(a).is_ok()) {
                            ui.selectable_value(&mut self.source, name.to_owned(), name);
                        }
                    });
                ui.add(
                    crate::tokens::input(
                        egui::TextEdit::singleline(&mut self.source).char_limit(63),
                    )
                    .desired_width(ui.available_width()),
                );
            });
        }
        text_field(ui, "Colormap variable (optional)", &mut self.colormap, 63);
        if let Some(s) = &self.current {
            if !s.file_info.is_empty() {
                crate::tokens::helper(ui, &s.file_info);
            }
            crate::tokens::helper(
                ui,
                format!(
                    "Source: {} rows × {} columns",
                    s.source_size[0], s.source_size[1]
                ),
            );
        }
        ui.add_space(crate::tokens::S);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(self.running, egui::Button::new("Stop"))
                .clicked()
            {
                self.stop_requested = true;
            }
            if ui
                .add_enabled(self.stopped, egui::Button::new("Resume"))
                .clicked()
            {
                self.stopped = false;
            }
        });
        ui.add_space(crate::tokens::S);
        crate::tokens::helper(
            ui,
            "Drag to pan, crop or measure. The wheel zooms around the cursor.",
        );
        ui.horizontal_wrapped(|ui| {
            for (tool, label) in [
                (Tool::Pan, "Pan"),
                (Tool::Crop, "Crop"),
                (Tool::Distance, "Distance"),
                (Tool::Contrast, "Contrast"),
            ] {
                if crate::tokens::toggle(ui, self.tool == tool, label).clicked() {
                    self.tool = tool;
                }
            }
        });
        ui.add_space(crate::tokens::S);
        let mut use_crop = self.crop.is_some();
        if ui.checkbox(&mut use_crop, "Crop image").changed() {
            self.crop = use_crop.then(|| {
                self.current.as_ref().map_or(
                    Crop {
                        x: 1,
                        y: 1,
                        width: 0,
                        height: 0,
                    },
                    |s| Crop {
                        x: 1,
                        y: 1,
                        width: s.source_size[1] - 1,
                        height: s.source_size[0] - 1,
                    },
                )
            });
        }
        if let Some(c) = &mut self.crop {
            egui::Grid::new("iv_crop_numbers")
                .spacing(egui::vec2(crate::tokens::S, crate::tokens::XS))
                .show(ui, |ui| {
                    for (label, value, lo) in [
                        ("Column x", &mut c.x, 1),
                        ("Row y", &mut c.y, 1),
                        ("Width", &mut c.width, 0),
                        ("Height", &mut c.height, 0),
                    ] {
                        ui.label(label);
                        ui.add(
                            egui::DragValue::new(value).range(lo..=matrust::matrix::MAX_ELEMENTS),
                        );
                        ui.end_row();
                    }
                });
            crate::tokens::helper(
                ui,
                "Includes both endpoints: width + 1 columns, height + 1 rows.",
            );
        }
        ui.add_space(crate::tokens::S);
        ui.horizontal_wrapped(|ui| {
            if ui.button("Original contrast").clicked() {
                self.contrast = Contrast::Original;
            }
            if ui.button("Match data range").clicked() {
                self.contrast = Contrast::DataRange;
                self.tool = Tool::Contrast;
            }
            if ui.button("Saturate 1 %").clicked() {
                self.contrast = Contrast::Saturate;
                self.tool = Tool::Contrast;
            }
        });
        for (i, label) in ["Window minimum", "Window maximum"].iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(*label);
                if ui
                    .add(
                        egui::DragValue::new(&mut self.window[i])
                            .range(0.0..=1.0)
                            .speed(0.001)
                            .max_decimals(6),
                    )
                    .changed()
                {
                    self.contrast = Contrast::Window;
                }
            });
        }
        crate::tokens::helper(
            ui,
            "Contrast window is normalized to the image class range.",
        );
        if ui.button("Clear distances").clicked() {
            self.lines.clear();
        }
    }
    fn main_caption(&self) -> &'static str {
        "IMAGE"
    }
    fn main_ui(&mut self, ui: &mut egui::Ui) {
        self.image_ui(ui);
    }
    fn results(&self) -> Vec<(&'static str, String)> {
        let Some(s) = &self.current else {
            return Vec::new();
        };
        vec![
            (
                "Shown image",
                format!(
                    "{} × {} · {}",
                    s.image.rows(),
                    s.image.dims()[1],
                    s.image.class()
                ),
            ),
            (
                "Contrast limits",
                s.limits
                    .chunks(2)
                    .map(|v| format!("[{} {}]", short(v[0]), short(v[1])))
                    .collect::<Vec<_>>()
                    .join("; "),
            ),
            (
                "Crop [x y w h]",
                s.crop.map_or_else(|| "Full image".into(), Crop::vector),
            ),
            ("Distances", s.distances.len().to_string()),
        ]
    }
    fn names_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        self.histogram_ui(ui);
        if let Some(s) = &self.current {
            if !s.distances.is_empty() {
                crate::tokens::helper(
                    ui,
                    "Distances use source pixels; positive angles point down.",
                );
            }
            for (i, d) in s.distances.iter().enumerate() {
                crate::tokens::helper(
                    ui,
                    format!(
                        "{}: ({}, {}) → ({}, {}) · {} px · {}°",
                        i + 1,
                        short(d[0]),
                        short(d[1]),
                        short(d[2]),
                        short(d[3]),
                        short(d[4]),
                        short(d[5])
                    ),
                );
            }
        }
        ui.add_space(crate::tokens::S);
        name_field(ui, "Image name", &mut self.image_name, ws);
        name_field(ui, "Distance list name", &mut self.distance_name, ws);
        name_field(
            ui,
            "Colormap name (empty for gray/RGB)",
            &mut self.map_name,
            ws,
        );
        ui.checkbox(&mut self.replace, "Replace existing variables");
    }
    fn exported(&self, _ws: &Snapshot) -> Vec<String> {
        vec![
            self.image_name.clone(),
            self.distance_name.clone(),
            self.map_name.clone(),
        ]
    }
    fn summary(&self) -> Option<String> {
        self.current.as_ref().map(|s| {
            format!(
                "Image Viewer: {} × {} · {} · {} distances",
                s.image.rows(),
                s.image.dims()[1],
                s.image.class(),
                s.distances.len()
            )
        })
    }
}
fn text_field(ui: &mut egui::Ui, label: &str, text: &mut String, limit: usize) {
    field(ui, label, |ui| {
        ui.add(
            crate::tokens::input(egui::TextEdit::singleline(text).char_limit(limit))
                .desired_width(ui.available_width()),
        );
    });
}

impl ImageViewer {
    fn image_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("Fit").clicked() {
                self.zoom = None;
                self.pan = egui::Vec2::ZERO;
            }
            if ui.button("100 %").clicked() {
                self.zoom = Some(1. / ui.ctx().pixels_per_point());
                self.pan = egui::Vec2::ZERO;
            }
            if let Some(z) = self.zoom {
                ui.label(format!("{:.0} %", z * ui.ctx().pixels_per_point() * 100.));
            }
        });
        ui.add_space(crate::tokens::S);
        let Some(shown) = &self.current else {
            crate::tokens::helper(
                ui,
                "Choose an image in INPUTS. The status line shows any error.",
            );
            return;
        };
        let texture = match &self.texture {
            Some((key, t)) if *key == shown.key => t.clone(),
            _ => {
                let [h, w, channels] = image_shape(&shown.display).expect("validated display");
                let Array::Typed(a) = &shown.display else {
                    unreachable!()
                };
                let TypedData::Uint8(data) = a.data() else {
                    unreachable!()
                };
                let mut rgb = Vec::with_capacity(h * w * 3);
                for row in 0..h {
                    for col in 0..w {
                        let at = row + col * h;
                        if channels == 3 {
                            rgb.extend([data[at], data[at + h * w], data[at + 2 * h * w]]);
                        } else {
                            rgb.extend([data[at]; 3]);
                        }
                    }
                }
                let t = ui.ctx().load_texture(
                    "image-viewer",
                    egui::ColorImage::from_rgb([w, h], &rgb),
                    egui::TextureOptions::NEAREST,
                );
                self.texture = Some((shown.key, t.clone()));
                t
            }
        };
        let p = crate::tokens::palette(ui.ctx());
        let available = egui::vec2(
            ui.available_width().max(8.),
            (ui.available_height() - 32.).max(96.),
        );
        let (viewport, response) = ui.allocate_exact_size(available, egui::Sense::click_and_drag());
        let painter = ui.painter_at(viewport);
        painter.rect_filled(viewport, 0., p.surface_sunken);
        let (h, w) = (shown.image.rows(), shown.image.dims()[1]);
        let native = egui::vec2(w as f32, h as f32);
        let fit = (viewport.width() / native.x)
            .min(viewport.height() / native.y)
            .max(0.00001);
        let mut scale = self.zoom.unwrap_or(fit);
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0. {
                ui.input_mut(|input| input.smooth_scroll_delta = egui::Vec2::ZERO);
                let next = (scale * (scroll * 0.002).exp()).clamp(fit.min(0.01), fit.max(1.) * 64.);
                if let Some(pos) = response.hover_pos() {
                    self.pan = (self.pan + (viewport.center() - pos)) * (next / scale)
                        - (viewport.center() - pos);
                }
                self.zoom = Some(next);
                scale = next;
            }
        }
        if self.tool == Tool::Pan && response.dragged() {
            self.pan += response.drag_delta();
        }
        // Keep at least one image edge within reach, even after a resize/crop.
        let extent = native * scale;
        let reach_x = ((extent.x + viewport.width()) / 2. - 8.).max(0.);
        let reach_y = ((extent.y + viewport.height()) / 2. - 8.).max(0.);
        self.pan.x = self.pan.x.clamp(-reach_x, reach_x);
        self.pan.y = self.pan.y.clamp(-reach_y, reach_y);
        let image_rect = egui::Rect::from_center_size(viewport.center() + self.pan, extent);
        let uv = egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.));
        painter.image(texture.id(), image_rect, uv, egui::Color32::WHITE);
        let offset = shown.crop.map_or([0, 0], |c| [c.x - 1, c.y - 1]);
        let point = |pos: egui::Pos2| -> [usize; 2] {
            let q = (pos - image_rect.min) / scale;
            [
                (q.x.floor() as i64 + 1).clamp(1, w as i64) as usize + offset[0],
                (q.y.floor() as i64 + 1).clamp(1, h as i64) as usize + offset[1],
            ]
        };
        let screen = |point: [f64; 2]| {
            image_rect.min
                + egui::vec2(
                    (point[0] - offset[0] as f64 - 0.5) as f32,
                    (point[1] - offset[1] as f64 - 0.5) as f32,
                ) * scale
        };
        for d in &shown.distances {
            let a = screen([d[0], d[1]]);
            let b = screen([d[2], d[3]]);
            painter.line_segment([a, b], egui::Stroke::new(1., p.accent));
            painter.circle_stroke(a, 4., egui::Stroke::new(1., p.accent));
            painter.circle_stroke(b, 4., egui::Stroke::new(1., p.accent));
        }
        if matches!(self.tool, Tool::Crop | Tool::Distance) {
            if response.drag_started()
                && let Some(pos) = ui
                    .input(|i| i.pointer.press_origin())
                    .filter(|p| image_rect.contains(*p))
            {
                self.drag_start = Some(point(pos));
                self.drag_end = self.drag_start;
            }
            if response.dragged()
                && let Some(pos) = response.interact_pointer_pos()
            {
                self.drag_end = Some(point(pos));
            }
            if let (Some(a), Some(b)) = (self.drag_start, self.drag_end) {
                let a = screen(a.map(|v| v as f64));
                let b = screen(b.map(|v| v as f64));
                if self.tool == Tool::Crop {
                    painter.rect_stroke(
                        egui::Rect::from_two_pos(a, b),
                        0.,
                        egui::Stroke::new(1., p.accent),
                        egui::StrokeKind::Inside,
                    );
                } else {
                    painter.line_segment([a, b], egui::Stroke::new(1., p.accent));
                }
            }
            if response.drag_stopped()
                && let (Some(a), Some(b)) = (self.drag_start.take(), self.drag_end.take())
            {
                if self.tool == Tool::Crop {
                    self.crop = Some(Crop::from_points(a, b));
                    self.zoom = None;
                    self.pan = egui::Vec2::ZERO;
                } else if self.lines.len() < MAX_DISTANCES {
                    self.lines.push([a.map(|v| v as f64), b.map(|v| v as f64)]);
                }
            }
        } else {
            self.drag_start = None;
            self.drag_end = None;
        }
        // The overview always uses the same cached texture. Its viewport box
        // uses source geometry, independent of display downsampling.
        if extent.x > viewport.width() + 1. || extent.y > viewport.height() + 1. {
            let factor = (128. / native.x).min(96. / native.y);
            let size = native * factor;
            let inset = egui::Rect::from_min_size(
                viewport.right_top() + egui::vec2(-size.x - 8., 8.),
                size,
            );
            painter.rect_filled(inset.expand(4.), 0., p.surface);
            painter.image(texture.id(), inset, uv, egui::Color32::WHITE);
            let visible = viewport.intersect(image_rect);
            let rect = egui::Rect::from_min_max(
                inset.min + (visible.min - image_rect.min) * (factor / scale),
                inset.min + (visible.max - image_rect.min) * (factor / scale),
            );
            painter.rect_stroke(
                rect,
                0.,
                egui::Stroke::new(1., p.accent),
                egui::StrokeKind::Inside,
            );
        }
        let readout = response
            .hover_pos()
            .filter(|pos| image_rect.contains(*pos))
            .and_then(|pos| {
                let at = point(pos);
                pixel(
                    &shown.image,
                    &shown.map,
                    at[1] - offset[1],
                    at[0] - offset[0],
                )
            })
            .unwrap_or_else(|| "Hover over the image to inspect a pixel".into());
        crate::tokens::helper(ui, readout);
    }
    fn histogram_ui(&mut self, ui: &mut egui::Ui) {
        let Some(s) = &self.current else {
            return;
        };
        let p = crate::tokens::palette(ui.ctx());
        crate::tokens::helper(ui, "Histogram of shown image · normalized class range");
        let mut window = self.window;
        let mut changed = false;
        egui_plot::Plot::new("iv_histogram")
            .height(144.)
            .show_background(false)
            .allow_zoom(false)
            .allow_scroll(false)
            .allow_drag(false)
            .allow_boxed_zoom(false)
            .allow_axis_zoom_drag(false)
            .allow_double_click_reset(false)
            .show(ui, |plot| {
                for (c, counts) in s.histogram.iter().enumerate() {
                    let points: Vec<_> = counts
                        .iter()
                        .enumerate()
                        .map(|(i, n)| [i as f64 / 255., *n])
                        .collect();
                    let label = if s.histogram.len() == 1 {
                        "Gray"
                    } else {
                        ["R", "G", "B"][c]
                    };
                    let mut line = egui_plot::Line::new(label, points).color(p.text).width(1.);
                    if c == 1 {
                        line = line.style(egui_plot::LineStyle::dashed_dense());
                    }
                    if c == 2 {
                        line = line.style(egui_plot::LineStyle::dotted_dense());
                    }
                    plot.line(line);
                }
                for (i, v) in s.limits.iter().enumerate() {
                    plot.vline(
                        egui_plot::VLine::new(if i % 2 == 0 { "Minimum" } else { "Maximum" }, *v)
                            .color(p.accent),
                    );
                }
                if self.tool == Tool::Contrast
                    && plot.response().dragged()
                    && let Some(pos) = plot.pointer_coordinate()
                {
                    let x = pos.x.clamp(0., 1.);
                    let i = if (x - window[0]).abs() < (x - window[1]).abs() {
                        0
                    } else {
                        1
                    };
                    window[i] = x;
                    if window[0] < window[1] {
                        changed = true;
                    }
                }
            });
        if changed {
            self.window = window;
            self.contrast = Contrast::Window;
        }
        crate::tokens::helper(
            ui,
            "R solid · G dashed · B dotted. Contrast tool: drag either limit.",
        );
    }
    #[cfg(test)]
    pub fn texture_id(&self) -> Option<egui::TextureId> {
        self.texture.as_ref().map(|(_, t)| t.id())
    }
}

/// The only addition to Host's normal layout is a Stop / Resume control.
pub(crate) fn show(
    host: &mut Host<ImageViewer>,
    ui: &mut egui::Ui,
    ws: &Snapshot,
    busy: bool,
    action: &mut Option<crate::toolkits::Action>,
) {
    host.set_folder(ws.folder.as_deref());
    host.app.running = host.running();
    host.show(ui, ws, busy, action);
    if std::mem::take(&mut host.app.stop_requested) {
        stop(host);
    }
}

/// Cancel the private job and preserve its last successful view until Resume.
pub(crate) fn stop(host: &mut Host<ImageViewer>) {
    host.runner.stop();
    host.delivered = None;
    host.app.stopped = true;
}
