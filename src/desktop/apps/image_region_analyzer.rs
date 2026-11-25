//! Image Region Analyzer: connected regions of a binary image, measured with
//! regionprops and filtered with bwpropfilt and bwareafilt. One script feeds
//! the private preview, Export and Generate script (a local function
//! `[BW_out, properties] = filterRegions(BW_in)`); Rust validates the inputs,
//! shows the engine's label2rgb image and sorts the table it returns.
use super::{
    App, AppKind, Outputs, Request, Runner, Snapshot, field, name_field, number, short, unused_name,
};
use crate::theme::ThemedScrollArea;
use eframe::egui;
use matrust::Array;
use std::collections::BTreeSet;
use std::hash::{Hash, Hasher};

/// Longest displayed side; larger images are shown every `step`-th pixel.
/// All computation uses the full image.
pub const DISPLAY_MAX: usize = 2048;
/// Largest "keep n largest" count (the engine's region limit).
pub const MAX_KEEP: usize = 65536;
const MAX_PATH: usize = 1024;

/// Region properties the engine's regionprops measures, in table order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Property {
    Area,
    Perimeter,
    Centroid,
    Eccentricity,
    EquivDiameter,
    Extent,
    MajorAxisLength,
    MinorAxisLength,
    Orientation,
    FilledArea,
}
impl Property {
    pub const ALL: [Self; 10] = [
        Self::Area,
        Self::Perimeter,
        Self::Centroid,
        Self::Eccentricity,
        Self::EquivDiameter,
        Self::Extent,
        Self::MajorAxisLength,
        Self::MinorAxisLength,
        Self::Orientation,
        Self::FilledArea,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Area => "Area",
            Self::Perimeter => "Perimeter",
            Self::Centroid => "Centroid",
            Self::Eccentricity => "Eccentricity",
            Self::EquivDiameter => "EquivDiameter",
            Self::Extent => "Extent",
            Self::MajorAxisLength => "MajorAxisLength",
            Self::MinorAxisLength => "MinorAxisLength",
            Self::Orientation => "Orientation",
            Self::FilledArea => "FilledArea",
        }
    }
    /// Table columns per region (Centroid is [x y]).
    pub fn width(self) -> usize {
        if self == Self::Centroid { 2 } else { 1 }
    }
    /// bwpropfilt takes scalar properties only.
    pub fn filterable(self) -> bool {
        self != Self::Centroid
    }
}
/// Properties of the request the engine's regionprops does not measure yet.
pub const UNAVAILABLE: [&str; 3] = ["EulerNumber", "Solidity", "ConvexArea"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    Workspace,
    File,
}

/// A bwpropfilt range; an empty bound is open.
#[derive(Clone, Debug, Hash)]
pub struct Filter {
    pub property: Property,
    pub enabled: bool,
    pub min: String,
    pub max: String,
}
impl Filter {
    fn range(&self) -> Result<(String, String), String> {
        let what = self.property.name();
        let lo = bound(&self.min, &format!("{what} min"), f64::NEG_INFINITY)?;
        let hi = bound(&self.max, &format!("{what} max"), f64::INFINITY)?;
        if lo > hi {
            return Err(format!("{what}: min must not exceed max"));
        }
        Ok((literal(lo), literal(hi)))
    }
}

/// One finite or infinite numeric literal; empty means `open`.
fn bound(text: &str, what: &str, open: f64) -> Result<f64, String> {
    if text.trim().is_empty() {
        return Ok(open);
    }
    if text.len() > 64 {
        return Err(format!("{what}: at most 64 characters"));
    }
    super::single_expression(text).map_err(|e| format!("{what}: {e}"))?;
    let t = text.trim();
    let v = match t {
        "Inf" | "inf" => f64::INFINITY,
        "-Inf" | "-inf" => f64::NEG_INFINITY,
        _ => t
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite())
            .ok_or_else(|| format!("{what}: enter one numeric literal (or Inf)"))?,
    };
    Ok(v)
}
fn literal(v: f64) -> String {
    if v == f64::INFINITY {
        "Inf".into()
    } else if v == f64::NEG_INFINITY {
        "-Inf".into()
    } else {
        number(v)
    }
}
fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// Class and colour layout of a workspace image the app can read.
pub fn image_info(a: &Array) -> Option<(&'static str, bool)> {
    if !matches!(a, Array::Numeric(_) | Array::Logical(_) | Array::Typed(_))
        || a.is_complex()
        || a.is_empty()
    {
        return None;
    }
    let class = a.class();
    if !matches!(class, "logical" | "double" | "single" | "uint8" | "uint16") {
        return None;
    }
    match a.dims().as_ref() {
        [_, _] => Some((class, false)),
        [_, _, 3] => Some((class, true)),
        _ => None,
    }
}

/// A delivered run: the label matrix of the kept regions, its display
/// image and the property table.
pub struct Analysis {
    pub rows: usize,
    pub cols: usize,
    /// Label matrix of the output image (shared storage, full size).
    labels: Array,
    pub count: usize,
    /// Display sampling: every `step`-th row and column.
    pub step: usize,
    /// Display image: width, height, RGB bytes from the top row.
    view: (usize, usize, Vec<u8>),
    /// Table columns: n x width, column-major.
    pub columns: Vec<(Property, Vec<f64>)>,
    generation: u64,
}
impl Analysis {
    /// Region (0-based) at a full-size pixel, if any.
    pub fn region_at(&self, row: usize, col: usize) -> Option<usize> {
        let v = self.labels.scalar(row, col).ok()?.into_real().ok()?;
        (v >= 1. && v as usize <= self.count).then(|| v as usize - 1)
    }
    /// Value of `property` (component `part`) for `region`.
    pub fn value(&self, property: Property, region: usize, part: usize) -> Option<f64> {
        let (_, data) = self.columns.iter().find(|(p, _)| *p == property)?;
        data.get(part * self.count + region).copied()
    }
}

pub struct ImageRegionAnalyzer {
    pub source: Source,
    pub variable: String,
    pub file: String,
    pub binarize: bool,
    pub connectivity: u8,
    pub fill_holes: bool,
    pub clear_border: bool,
    /// Table columns, by `Property::ALL` position.
    pub shown: [bool; 10],
    pub filters: Vec<Filter>,
    pub keep_largest: bool,
    pub keep_count: String,
    pub bw_name: String,
    pub table_name: String,
    /// Last good result; errors keep it on screen.
    pub current: Option<Analysis>,
    /// Selected region, 0-based (row of the unsorted table).
    pub selected: Option<usize>,
    /// Sort column (property, part) and descending.
    pub sort: Option<(Property, usize, bool)>,
    /// The host's runner state, once per frame (shows Stop).
    pub running: bool,
    stopped: Option<u64>,
    generation: u64,
    scroll_to_selected: bool,
    texture: Option<(u64, egui::TextureHandle)>,
    outline_runner: Runner,
    /// Outline of the selected region: request key and closed (x, y) chain.
    outline: Option<(u64, Vec<[f64; 2]>)>,
}
impl Default for ImageRegionAnalyzer {
    fn default() -> Self {
        Self {
            source: Source::Workspace,
            variable: String::new(),
            file: String::new(),
            binarize: false,
            connectivity: 8,
            fill_holes: false,
            clear_border: false,
            shown: [
                true, true, true, true, true, true, false, false, false, false,
            ],
            filters: Property::ALL
                .into_iter()
                .filter(|p| p.filterable())
                .map(|property| Filter {
                    property,
                    enabled: false,
                    min: String::new(),
                    max: String::new(),
                })
                .collect(),
            keep_largest: false,
            keep_count: "1".into(),
            bw_name: "BW_out".into(),
            table_name: "properties".into(),
            current: None,
            selected: None,
            sort: None,
            running: false,
            stopped: None,
            generation: 0,
            scroll_to_selected: false,
            texture: None,
            outline_runner: Runner::default(),
            outline: None,
        }
    }
}

struct Plan {
    script: String,
    bw: String,
    bw_out: String,
    table: String,
    columns: Vec<Property>,
}

impl ImageRegionAnalyzer {
    pub fn columns(&self) -> Vec<Property> {
        Property::ALL
            .into_iter()
            .zip(self.shown)
            .filter_map(|(p, shown)| shown.then_some(p))
            .collect()
    }
    /// The filter of `property` (tests and scripted setups).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn filter_mut(&mut self, property: Property) -> Option<&mut Filter> {
        self.filters.iter_mut().find(|f| f.property == property)
    }
    /// Everything the script depends on, for Stop.
    fn settings_key(&self) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (
            self.source,
            &self.variable,
            &self.file,
            self.binarize,
            self.connectivity,
            self.fill_holes,
            self.clear_border,
            self.shown,
        )
            .hash(&mut h);
        (
            &self.filters,
            self.keep_largest,
            &self.keep_count,
            &self.bw_name,
            &self.table_name,
        )
            .hash(&mut h);
        h.finish()
    }
    /// Stop: cancels the running analysis; none starts until an input
    /// changes or Run is pressed.
    pub fn stop(&mut self) {
        self.stopped = Some(self.settings_key());
    }
    pub fn run(&mut self) {
        self.stopped = None;
    }
    pub fn is_stopped(&self) -> bool {
        self.stopped == Some(self.settings_key())
    }
    /// The image the script starts from, checked.
    fn source_expression(&self, ws: &Snapshot) -> Result<String, String> {
        match self.source {
            Source::Workspace => {
                let v = self.variable.trim();
                if v.is_empty() {
                    return Err("Image Region Analyzer: choose an image variable".into());
                }
                super::single_expression(v)?;
                if v.len() > 63 || !matrust::files::valid_name(v) {
                    return Err("Image Region Analyzer: choose an image variable".into());
                }
                let a = ws
                    .get(v)
                    .ok_or_else(|| format!("Image Region Analyzer: {v} no longer exists"))?;
                let Some((class, rgb)) = image_info(a) else {
                    return Err(format!(
                        "Image Region Analyzer: {v} is not a real 2-D or RGB image \
                         (logical, double, single, uint8 or uint16)"
                    ));
                };
                if !self.binarize && (class != "logical" || rgb) {
                    return Err(format!(
                        "Image Region Analyzer: {v} is {class}{}, not a logical mask; \
                         tick Binarize (Otsu) to threshold it",
                        if rgb { " RGB" } else { "" }
                    ));
                }
                Ok(v.to_owned())
            }
            Source::File => {
                let path = self.file.trim();
                if path.is_empty() {
                    return Err("Image Region Analyzer: enter an image file".into());
                }
                if path.len() > MAX_PATH || path.chars().any(char::is_control) {
                    return Err(format!(
                        "Image Region Analyzer: the file name must be one line of at most {MAX_PATH} characters"
                    ));
                }
                let call = format!("imread({})", quote(path));
                super::single_expression(&call)?;
                Ok(call)
            }
        }
    }
    fn output_names(&self) -> Result<(), String> {
        for name in [&self.bw_name, &self.table_name] {
            super::single_expression(name)?;
            if name.len() > 63 || !matrust::files::valid_name(name) {
                return Err(
                    "Image Region Analyzer: use valid variable names (at most 63 characters)"
                        .into(),
                );
            }
            super::check_export_name(name)?;
        }
        if self.bw_name == self.table_name {
            return Err(format!(
                "Image Region Analyzer: duplicate export name {}",
                self.bw_name
            ));
        }
        Ok(())
    }
    fn keep(&self) -> Result<Option<usize>, String> {
        if !self.keep_largest {
            return Ok(None);
        }
        let what = "Keep n largest";
        if self.keep_count.len() > 64 {
            return Err(format!("{what}: at most 64 characters"));
        }
        super::single_expression(&self.keep_count).map_err(|e| format!("{what}: {e}"))?;
        match self.keep_count.trim().parse::<f64>() {
            Ok(n) if n >= 1. && n <= MAX_KEEP as f64 && n.fract() == 0. => Ok(Some(n as usize)),
            _ => Err(format!("{what}: enter an integer from 1 to {MAX_KEEP}")),
        }
    }
    fn plan(&self, ws: &Snapshot) -> Result<Plan, String> {
        self.output_names()?;
        let columns = self.columns();
        if columns.is_empty() {
            return Err("Image Region Analyzer: choose at least one table column".into());
        }
        if !matches!(self.connectivity, 4 | 8) {
            return Err("Image Region Analyzer: connectivity is 4 or 8".into());
        }
        let source = self.source_expression(ws)?;
        let mut filters = Vec::new();
        for f in self.filters.iter().filter(|f| f.enabled) {
            if !f.property.filterable() {
                return Err(format!("{} cannot be filtered", f.property.name()));
            }
            let (lo, hi) = f
                .range()
                .map_err(|e| format!("Image Region Analyzer: {e}"))?;
            filters.push((f.property, lo, hi));
        }
        let keep = self
            .keep()
            .map_err(|e| format!("Image Region Analyzer: {e}"))?;
        let mut taken: BTreeSet<String> = [self.bw_name.clone(), self.table_name.clone()].into();
        let mut fresh = |base: &str| {
            let name = unused_name(base, |n| ws.contains(n) || taken.contains(n));
            taken.insert(name.clone());
            name
        };
        let bw = fresh("ira_bw");
        let function = fresh("filterRegions");
        let conn = self.connectivity;
        let mut script = format!(
            "% Image Region Analyzer: {conn}-connected regions of a binary image, filtered,\n\
             % then measured with regionprops (one row per region of {}).\n\
             {bw} = {source};\n",
            self.bw_name
        );
        if self.binarize {
            script.push_str(&format!(
                "if ~strcmp(class({bw}), 'logical')\n    \
                 if ndims({bw}) == 3 && size({bw}, 3) == 3, {bw} = rgb2gray({bw}); end\n    \
                 {bw} = imbinarize({bw}); % global Otsu threshold\n\
                 end\n"
            ));
        }
        script.push_str(&format!(
            "if ~strcmp(class({bw}), 'logical') || ndims({bw}) ~= 2\n    \
             error('Image Region Analyzer: the source must be a 2-D logical mask; tick Binarize (Otsu) for other images');\n\
             end\n\
             [{}, {}] = {function}({bw});\n",
            self.bw_name, self.table_name
        ));
        // The local function: the steps of the app, reusable on other masks.
        script.push_str(&format!(
            "\nfunction [BW_out, properties] = {function}(BW_in)\n\
             % Steps: {}.\n\
             conn = {conn};\n\
             BW_out = BW_in;\n",
            steps(self.fill_holes, self.clear_border, &filters, keep)
        ));
        if self.fill_holes {
            script.push_str("BW_out = imfill(BW_out, conn, 'holes');\n");
        }
        if self.clear_border {
            script.push_str("BW_out = imclearborder(BW_out, conn);\n");
        }
        for (property, lo, hi) in &filters {
            script.push_str(&format!(
                "BW_out = bwpropfilt(BW_out, '{}', [{lo} {hi}], conn);\n",
                property.name()
            ));
        }
        if let Some(n) = keep {
            script.push_str(&format!("BW_out = bwareafilt(BW_out, {n}, conn);\n"));
        }
        // One regionprops call per property keeps each struct within the
        // engine's element limit.
        script.push_str("cc = bwconncomp(BW_out, conn);\n");
        for p in &columns {
            let name = p.name();
            script.push_str(&format!(
                "s = regionprops(cc, '{name}');\n{name} = reshape([s.{name}], {}, [])';\n",
                p.width()
            ));
        }
        let names: Vec<&str> = columns.iter().map(|p| p.name()).collect();
        script.push_str(&format!(
            "properties = table({}, 'VariableNames', {{{}}});\nend\n",
            names.join(", "),
            names
                .iter()
                .map(|n| format!("'{n}'"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
        Ok(Plan {
            script,
            bw,
            bw_out: self.bw_name.clone(),
            table: self.table_name.clone(),
            columns,
        })
    }

    /// The private run that traces the selected region's outer boundary
    /// (bwboundaries on its bounding box), if a region is selected.
    pub fn outline_request(&self) -> Option<Request> {
        let a = self.current.as_ref()?;
        let k = self.selected? + 1;
        let code = format!(
            "[iraO_r, iraO_c] = find(iraO_L == {k});\n\
             iraO_r0 = min(iraO_r); iraO_c0 = min(iraO_c);\n\
             iraO_B = bwboundaries(iraO_L(iraO_r0:max(iraO_r), iraO_c0:max(iraO_c)) == {k}, 'noholes');\n\
             iraO_b = iraO_B{{1}};\n\
             iraO_xy = [iraO_b(:, 2) + iraO_c0 - 1, iraO_b(:, 1) + iraO_r0 - 1];\n"
        );
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (a.generation, k).hash(&mut h);
        Some(Request {
            key: h.finish(),
            inputs: vec![("iraO_L".into(), a.labels.clone())],
            code,
            outputs: vec!["iraO_xy".into()],
        })
    }
    /// Stores a finished outline run (x, y in full-size pixel coordinates).
    pub fn receive_outline(&mut self, key: u64, outputs: &Outputs) -> Result<(), String> {
        let m = match outputs.get("iraO_xy") {
            Some(Array::Numeric(m)) => m,
            _ => return Err("Image Region Analyzer: missing outline".into()),
        };
        let data = m.data();
        let n = m.rows();
        if m.cols() != 2 || data.len() != 2 * n {
            return Err("Image Region Analyzer: malformed outline".into());
        }
        self.outline = Some((key, (0..n).map(|i| [data[i], data[n + i]]).collect()));
        Ok(())
    }
    /// The outline of the current selection, once traced.
    pub fn outline(&self) -> Option<&[[f64; 2]]> {
        let key = self.outline_request()?.key;
        self.outline
            .as_ref()
            .filter(|(k, _)| *k == key)
            .map(|(_, points)| points.as_slice())
    }
    fn poll_outline(&mut self, ctx: &egui::Context) {
        let Some(request) = self.outline_request() else {
            self.outline_runner.stop();
            return;
        };
        let key = request.key;
        if self.outline.as_ref().is_some_and(|(k, _)| *k == key) {
            return;
        }
        self.outline_runner.submit(ctx, request);
        if self.outline_runner.key() == Some(key) {
            match self.outline_runner.result() {
                Some(Ok(outputs)) => {
                    let outputs = outputs.clone();
                    if self.receive_outline(key, &outputs).is_err() {
                        self.outline = Some((key, Vec::new()));
                    }
                }
                // A region too large to trace keeps no outline.
                Some(Err(_)) => self.outline = Some((key, Vec::new())),
                None => {}
            }
        }
    }

    /// Full-size (row, col) under `pos` of an image drawn in `rect`.
    pub fn pixel_at(&self, rect: egui::Rect, pos: egui::Pos2) -> Option<(usize, usize)> {
        let a = self.current.as_ref()?;
        if !rect.contains(pos) || rect.width() <= 0. || rect.height() <= 0. {
            return None;
        }
        let fx = ((pos.x - rect.left()) / rect.width()) as f64;
        let fy = ((pos.y - rect.top()) / rect.height()) as f64;
        let col = ((fx * a.cols as f64).floor() as usize).min(a.cols - 1);
        let row = ((fy * a.rows as f64).floor() as usize).min(a.rows - 1);
        Some((row, col))
    }
    /// A click on the image: the region under it becomes the selected row
    /// (background clears the selection).
    pub fn select_at(&mut self, rect: egui::Rect, pos: egui::Pos2) -> Option<usize> {
        let (row, col) = self.pixel_at(rect, pos)?;
        let region = self.current.as_ref()?.region_at(row, col);
        self.selected = region;
        self.scroll_to_selected = region.is_some();
        region
    }
    /// Table rows (regions, 0-based) in display order.
    pub fn sorted_rows(&self) -> Vec<usize> {
        let Some(a) = &self.current else {
            return Vec::new();
        };
        let mut rows: Vec<usize> = (0..a.count).collect();
        if let Some((p, part, descending)) = self.sort {
            let value = |r: usize| a.value(p, r, part).unwrap_or(f64::NAN);
            rows.sort_by(|&i, &j| {
                let (x, y) = (value(i), value(j));
                // NaN last in both directions; ties keep region order.
                match (x.is_nan(), y.is_nan()) {
                    (true, true) => std::cmp::Ordering::Equal,
                    (true, false) => std::cmp::Ordering::Greater,
                    (false, true) => std::cmp::Ordering::Less,
                    _ if descending => y.total_cmp(&x),
                    _ => x.total_cmp(&y),
                }
                .then(i.cmp(&j))
            });
        }
        rows
    }
    /// Display position of the selected region in the sorted table.
    pub fn selected_row(&self) -> Option<usize> {
        let s = self.selected?;
        self.sorted_rows().iter().position(|&r| r == s)
    }
    /// Header click: ascending, then descending, then unsorted.
    pub fn toggle_sort(&mut self, property: Property, part: usize) {
        self.sort = match self.sort {
            Some((p, k, false)) if p == property && k == part => Some((p, k, true)),
            Some((p, k, true)) if p == property && k == part => None,
            _ => Some((property, part, false)),
        };
    }
}

fn steps(
    fill: bool,
    clear: bool,
    filters: &[(Property, String, String)],
    keep: Option<usize>,
) -> String {
    let mut out = Vec::new();
    if fill {
        out.push("fill holes".to_owned());
    }
    if clear {
        out.push("remove regions touching the border".to_owned());
    }
    for (p, lo, hi) in filters {
        out.push(format!("keep {} in [{lo}, {hi}]", p.name()));
    }
    if let Some(n) = keep {
        out.push(format!("keep the {n} largest by Area"));
    }
    if out.is_empty() {
        "none (all regions)".into()
    } else {
        out.join("; ")
    }
}

fn numeric(outputs: &Outputs, name: &str) -> Result<Vec<f64>, String> {
    match outputs.get(name) {
        Some(Array::Numeric(m)) => Ok(m.data().to_vec()),
        Some(a @ (Array::Typed(_) | Array::Logical(_))) => a
            .to_numeric(&std::sync::atomic::AtomicBool::new(false))
            .map(|m| m.data().to_vec()),
        _ => Err(format!(
            "Image Region Analyzer: missing numeric result {name}"
        )),
    }
}

impl App for ImageRegionAnalyzer {
    fn kind(&self) -> AppKind {
        AppKind::ImageRegionAnalyzer
    }
    fn script(&self, ws: &Snapshot) -> Result<String, String> {
        if self.is_stopped() {
            return Err("Image Region Analyzer: stopped. Change an input or press Run.".into());
        }
        Ok(self.plan(ws)?.script)
    }
    fn inputs(&self) -> Vec<String> {
        match self.source {
            Source::Workspace if !self.variable.trim().is_empty() => {
                vec![self.variable.trim().to_owned()]
            }
            _ => Vec::new(),
        }
    }
    fn prelude(&self, ws: &Snapshot) -> String {
        if self.source != Source::File {
            return String::new();
        }
        let mut code = String::new();
        let mut path = std::path::PathBuf::from(self.file.trim());
        if let Some(folder) = ws.folder() {
            // Relative file names resolve as in the workspace.
            code.push_str(&format!(
                "try\ncd({});\nend\n",
                quote(&folder.to_string_lossy())
            ));
            path = folder.join(path);
        }
        // A changed file reruns the analysis.
        if let Ok(modified) = std::fs::metadata(&path).and_then(|m| m.modified()) {
            code.push_str(&format!("% file modified {modified:?}\n"));
        }
        code
    }
    fn probe(&self, ws: &Snapshot) -> (String, Vec<String>) {
        let Ok(plan) = self.plan(ws) else {
            return (String::new(), Vec::new());
        };
        let (bw, out, conn) = (&plan.bw, &plan.bw_out, self.connectivity);
        let mut code = format!(
            "iraP_L = labelmatrix(bwconncomp({out}, {conn}));\n\
             iraP_step = max(1, ceil(max(size(iraP_L)) / {DISPLAY_MAX}));\n\
             iraP_view = label2rgb(iraP_L(1:iraP_step:end, 1:iraP_step:end), 'jet', 'w');\n\
             iraP_removed = {bw}(1:iraP_step:end, 1:iraP_step:end) & ~{out}(1:iraP_step:end, 1:iraP_step:end);\n\
             iraP_view(cat(3, iraP_removed, iraP_removed, iraP_removed)) = 190;\n"
        );
        let mut outputs = vec!["iraP_L".into(), "iraP_step".into(), "iraP_view".into()];
        for p in &plan.columns {
            let name = format!("iraP_col_{}", p.name());
            code.push_str(&format!("{name} = {}.{};\n", plan.table, p.name()));
            outputs.push(name);
        }
        (code, outputs)
    }
    fn receive(&mut self, outputs: &Outputs) -> Result<(), String> {
        let labels = outputs
            .get("iraP_L")
            .cloned()
            .ok_or("Image Region Analyzer: missing label matrix")?;
        let (rows, cols) = match labels.dims().as_ref() {
            [r, c] => (*r, *c),
            _ => return Err("Image Region Analyzer: the label matrix is not 2-D".into()),
        };
        let step = numeric(outputs, "iraP_step")?
            .first()
            .copied()
            .filter(|s| *s >= 1. && s.fract() == 0.)
            .ok_or("Image Region Analyzer: bad display step")? as usize;
        let view = match outputs.get("iraP_view") {
            Some(Array::Typed(t)) if t.class().name() == "uint8" => {
                let d = t.dims();
                let (h, w) = match d.as_ref() {
                    [h, w, 3] => (*h, *w),
                    [h, w] if *h == 0 || *w == 0 => (*h, *w),
                    _ => return Err("Image Region Analyzer: bad display image".into()),
                };
                let bytes = t.to_le_bytes();
                let mut rgb = Vec::with_capacity(h * w * 3);
                for r in 0..h {
                    for c in 0..w {
                        for k in 0..3 {
                            rgb.push(bytes[k * h * w + c * h + r]);
                        }
                    }
                }
                (w, h, rgb)
            }
            _ => return Err("Image Region Analyzer: missing display image".into()),
        };
        let columns = self.columns();
        let mut table = Vec::new();
        let mut count = None;
        for p in columns {
            let data = numeric(outputs, &format!("iraP_col_{}", p.name()))?;
            let n = data.len() / p.width();
            if data.len() != n * p.width() || count.is_some_and(|c| c != n) {
                return Err("Image Region Analyzer: inconsistent table".into());
            }
            count = Some(n);
            table.push((p, data));
        }
        let count = count.unwrap_or(0);
        self.generation += 1;
        if self.selected.is_some_and(|s| s >= count) {
            self.selected = None;
        }
        self.current = Some(Analysis {
            rows,
            cols,
            labels,
            count,
            step,
            view,
            columns: table,
            generation: self.generation,
        });
        Ok(())
    }
    fn prepare(&mut self, _ws: &Snapshot) {
        if self.stopped.is_some() && !self.is_stopped() {
            self.stopped = None;
        }
    }
    fn inputs_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        field(ui, "Source", |ui| {
            ui.horizontal(|ui| {
                let r = ui.radio_value(&mut self.source, Source::Workspace, "Workspace");
                crate::focus_areas::register(
                    ui,
                    &r,
                    crate::focus_areas::Area::App(AppKind::ImageRegionAnalyzer),
                );
                ui.radio_value(&mut self.source, Source::File, "Image file");
            });
        });
        match self.source {
            Source::Workspace => field(ui, "Image variable", |ui| {
                egui::ComboBox::from_id_salt("ira_variable")
                    .width(ui.available_width())
                    .selected_text(if self.variable.is_empty() {
                        "Choose image"
                    } else {
                        &self.variable
                    })
                    .show_ui(ui, |ui| {
                        for (name, a) in ws.variables() {
                            if let Some((class, rgb)) = image_info(a) {
                                let d = a.dims();
                                let text = format!(
                                    "{name}  {}×{} {class}{}",
                                    d[0],
                                    d[1],
                                    if rgb { " RGB" } else { "" }
                                );
                                ui.selectable_value(&mut self.variable, name.to_owned(), text);
                            }
                        }
                    });
            }),
            Source::File => field(ui, "Image file (imread)", |ui| {
                ui.add(
                    crate::tokens::input(
                        egui::TextEdit::singleline(&mut self.file).char_limit(MAX_PATH),
                    )
                    .desired_width(ui.available_width()),
                );
            }),
        }
        super::expression_message(ui, self.source_expression(ws).map(|_| ()));
        ui.checkbox(&mut self.binarize, "Binarize (Otsu)")
            .on_hover_text(
                "Threshold non-logical images with imbinarize; RGB is converted to gray first",
            );
        ui.add_space(crate::tokens::S);
        field(ui, "Connectivity", |ui| {
            ui.horizontal(|ui| {
                ui.radio_value(&mut self.connectivity, 8, "8");
                ui.radio_value(&mut self.connectivity, 4, "4");
            });
        });
        ui.checkbox(&mut self.fill_holes, "Fill holes");
        ui.checkbox(&mut self.clear_border, "Clear border regions");
        ui.add_space(crate::tokens::S);
        if self.running && !self.is_stopped() {
            ui.horizontal(|ui| {
                crate::tokens::helper(ui, "Computing…");
                if ui.button("Stop").clicked() {
                    self.stop();
                }
            });
        } else if self.is_stopped() {
            ui.horizontal(|ui| {
                crate::tokens::helper(ui, "Stopped");
                if ui.button("Run").clicked() {
                    self.run();
                }
            });
        }
        crate::tokens::helper(
            ui,
            "Images above 2048 px a side are shown subsampled; all measurements use the full image.",
        );
    }
    fn main_caption(&self) -> &'static str {
        "REGIONS"
    }
    fn main_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.poll_outline(&ctx);
        let Some(a) = &self.current else {
            crate::tokens::helper(
                ui,
                "Choose a logical mask (or tick Binarize) to see its regions.",
            );
            return;
        };
        let (w, h) = (a.view.0, a.view.1);
        if w == 0 || h == 0 || a.rows == 0 || a.cols == 0 {
            crate::tokens::helper(ui, "The image is empty.");
            return;
        }
        let generation = a.generation;
        let texture = match &self.texture {
            Some((g, t)) if *g == generation => t.clone(),
            _ => {
                let t = ctx.load_texture(
                    "image-region-analyzer",
                    egui::ColorImage::from_rgb([w, h], &a.view.2),
                    egui::TextureOptions::NEAREST,
                );
                self.texture = Some((generation, t.clone()));
                t
            }
        };
        let p = crate::tokens::palette(&ctx);
        let (rows, cols, count, step) = (a.rows, a.cols, a.count, a.step);
        let caption = crate::tokens::CAPTION + crate::tokens::S;
        let avail = ui.available_size();
        let room = egui::vec2(avail.x.max(64.), (avail.y - caption).max(64.));
        let scale = (room.x / cols as f32).min(room.y / rows as f32);
        let size = egui::vec2(cols as f32 * scale, rows as f32 * scale);
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
        let painter = ui.painter_at(rect);
        painter.image(
            texture.id(),
            rect,
            egui::Rect::from_min_max(egui::pos2(0., 0.), egui::pos2(1., 1.)),
            egui::Color32::WHITE,
        );
        painter.rect_stroke(
            rect,
            0.,
            egui::Stroke::new(1., p.hairline),
            egui::StrokeKind::Inside,
        );
        if response.clicked()
            && let Some(pos) = response.interact_pointer_pos()
        {
            self.select_at(rect, pos);
        }
        let to_screen = |[x, y]: [f64; 2]| {
            egui::pos2(
                rect.left() + ((x - 0.5) as f32) * scale,
                rect.top() + ((y - 0.5) as f32) * scale,
            )
        };
        let stroke = egui::Stroke::new(2., p.accent);
        match self.outline() {
            Some([one]) => {
                painter.rect_stroke(
                    egui::Rect::from_center_size(
                        to_screen(*one),
                        egui::vec2(scale, scale).max(egui::vec2(4., 4.)),
                    ),
                    0.,
                    stroke,
                    egui::StrokeKind::Outside,
                );
            }
            Some(points) if points.len() > 1 => {
                painter.add(egui::Shape::closed_line(
                    points.iter().map(|&q| to_screen(q)).collect(),
                    stroke,
                ));
            }
            _ => {}
        }
        let mut text = format!("{rows}×{cols} px · {count} regions");
        if step > 1 {
            text.push_str(&format!(" · shown every {step}th pixel"));
        }
        text.push_str(" · click a region to select its row; grey: removed by the steps");
        crate::tokens::helper(ui, text);
    }
    fn results(&self) -> Vec<(&'static str, String)> {
        let Some(a) = &self.current else {
            return Vec::new();
        };
        let mut rows = vec![
            ("Image", format!("{}×{}", a.rows, a.cols)),
            ("Regions", a.count.to_string()),
        ];
        if let Some(s) = self.selected {
            rows.push(("Selected region", (s + 1).to_string()));
            if let Some(area) = a.value(Property::Area, s, 0) {
                rows.push(("Area", short(area)));
            }
        }
        rows
    }
    fn names_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        self.table_ui(ui);
        ui.add_space(crate::tokens::M);
        egui::CollapsingHeader::new("Columns")
            .id_salt("ira_columns")
            .show(ui, |ui| {
                for (k, p) in Property::ALL.into_iter().enumerate() {
                    ui.checkbox(&mut self.shown[k], p.name());
                }
                crate::tokens::helper(
                    ui,
                    format!(
                        "Not available in regionprops yet: {}",
                        UNAVAILABLE.join(", ")
                    ),
                );
            });
        egui::CollapsingHeader::new("Filters")
            .id_salt("ira_filters")
            .default_open(true)
            .show(ui, |ui| self.filters_ui(ui));
        ui.add_space(crate::tokens::S);
        name_field(ui, "Filtered mask", &mut self.bw_name, ws);
        name_field(ui, "Properties table", &mut self.table_name, ws);
        super::expression_message(ui, self.output_names());
    }
    fn exported(&self, _ws: &Snapshot) -> Vec<String> {
        vec![self.bw_name.clone(), self.table_name.clone()]
    }
    fn summary(&self) -> Option<String> {
        self.current.as_ref().map(|a| {
            format!(
                "Image Region Analyzer: {} regions · {}-connected",
                a.count, self.connectivity
            )
        })
    }
}

impl ImageRegionAnalyzer {
    fn filters_ui(&mut self, ui: &mut egui::Ui) {
        let half = ((ui.available_width() - crate::tokens::S) / 2.).max(48.);
        for f in &mut self.filters {
            ui.checkbox(&mut f.enabled, f.property.name());
            if f.enabled {
                ui.horizontal(|ui| {
                    ui.add(
                        crate::tokens::input(
                            egui::TextEdit::singleline(&mut f.min)
                                .char_limit(64)
                                .hint_text("min"),
                        )
                        .desired_width(half),
                    );
                    ui.add(
                        crate::tokens::input(
                            egui::TextEdit::singleline(&mut f.max)
                                .char_limit(64)
                                .hint_text("max"),
                        )
                        .desired_width(half),
                    );
                });
                super::expression_message(ui, f.range().map(|_| ()));
            }
        }
        ui.add_space(crate::tokens::XS);
        ui.checkbox(&mut self.keep_largest, "Keep n largest (by Area)");
        if self.keep_largest {
            ui.add(
                crate::tokens::input(
                    egui::TextEdit::singleline(&mut self.keep_count).char_limit(64),
                )
                .desired_width(ui.available_width()),
            );
            super::expression_message(ui, self.keep().map(|_| ()));
        }
        crate::tokens::helper(
            ui,
            "Ranges are inclusive; an empty bound is open. Filters apply in this order, then n largest.",
        );
    }

    fn table_ui(&mut self, ui: &mut egui::Ui) {
        let Some(a) = &self.current else {
            return;
        };
        let p = crate::tokens::palette(ui.ctx());
        let font = egui::FontId::proportional(crate::tokens::BODY);
        let row = crate::tokens::ROW;
        let index_w = 40.;
        let cell_w = 88.;
        // Display columns: (property, part, header).
        let mut cells = Vec::new();
        for (prop, _) in &a.columns {
            if *prop == Property::Centroid {
                cells.push((*prop, 0, "Centroid x".to_owned()));
                cells.push((*prop, 1, "Centroid y".to_owned()));
            } else {
                cells.push((*prop, 0, prop.name().to_owned()));
            }
        }
        let total = index_w + cell_w * cells.len() as f32;
        let order = self.sorted_rows();
        let count = a.count;
        let mut clicked_header = None;
        let mut clicked_row = None;
        let scroll_to = self
            .scroll_to_selected
            .then(|| self.selected_row())
            .flatten();
        egui::ScrollArea::horizontal()
            .id_salt("ira_table_x")
            .auto_shrink([false, true])
            .show_themed(ui, |ui| {
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.;
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(index_w, row), egui::Sense::hover());
                        ui.painter().text(
                            egui::pos2(rect.right() - crate::tokens::S, rect.center().y),
                            egui::Align2::RIGHT_CENTER,
                            "#",
                            font.clone(),
                            p.text_muted,
                        );
                        for (prop, part, header) in &cells {
                            let mark = match self.sort {
                                Some((sp, sk, d)) if sp == *prop && sk == *part => {
                                    if d {
                                        " ↓"
                                    } else {
                                        " ↑"
                                    }
                                }
                                _ => "",
                            };
                            let (rect, response) = ui
                                .allocate_exact_size(egui::vec2(cell_w, row), egui::Sense::click());
                            ui.painter().with_clip_rect(rect.shrink(2.)).text(
                                egui::pos2(rect.right() - crate::tokens::XS, rect.center().y),
                                egui::Align2::RIGHT_CENTER,
                                format!("{header}{mark}"),
                                font.clone(),
                                if mark.is_empty() {
                                    p.text_muted
                                } else {
                                    p.text
                                },
                            );
                            if response
                                .on_hover_text(format!("Sort by {header}"))
                                .clicked()
                            {
                                clicked_header = Some((*prop, *part));
                            }
                        }
                    });
                    let mut area = egui::ScrollArea::vertical()
                        .id_salt("ira_table_y")
                        .max_height(320.)
                        .auto_shrink([false, true]);
                    if let Some(at) = scroll_to {
                        area = area.vertical_scroll_offset(
                            (at as f32 * (row + ui.spacing().item_spacing.y) - 2. * row).max(0.),
                        );
                    }
                    area.show_rows_themed(ui, row, count, |ui, range| {
                        for &region in &order[range] {
                            let (rect, response) = ui
                                .allocate_exact_size(egui::vec2(total, row), egui::Sense::click());
                            let painter = ui.painter_at(rect);
                            if self.selected == Some(region) {
                                painter.rect_filled(rect, 2., p.accent_soft);
                            }
                            painter.text(
                                egui::pos2(
                                    rect.left() + index_w - crate::tokens::S,
                                    rect.center().y,
                                ),
                                egui::Align2::RIGHT_CENTER,
                                (region + 1).to_string(),
                                font.clone(),
                                p.text_muted,
                            );
                            for (j, (prop, part, _)) in cells.iter().enumerate() {
                                let cell = egui::Rect::from_min_size(
                                    egui::pos2(
                                        rect.left() + index_w + cell_w * j as f32,
                                        rect.top(),
                                    ),
                                    egui::vec2(cell_w, row),
                                );
                                let v = a.value(*prop, region, *part).unwrap_or(f64::NAN);
                                painter.with_clip_rect(cell.shrink(2.)).text(
                                    egui::pos2(cell.right() - crate::tokens::XS, cell.center().y),
                                    egui::Align2::RIGHT_CENTER,
                                    short(v),
                                    font.clone(),
                                    p.text,
                                );
                            }
                            if response.clicked() {
                                clicked_row = Some(region);
                            }
                        }
                    });
                });
            });
        if count == 0 {
            crate::tokens::helper(ui, "No regions remain.");
        }
        self.scroll_to_selected = false;
        if let Some((prop, part)) = clicked_header {
            self.toggle_sort(prop, part);
        }
        if let Some(region) = clicked_row {
            self.selected = Some(region);
        }
    }
}
