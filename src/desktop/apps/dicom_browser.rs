//! Clean-room folder/series browser. Public behavior reference:
//! https://www.mathworks.com/help/images/ref/dicombrowser-app.html
//! A single builder supplies private reads, workspace export and saved scripts.
use super::{App, AppKind, Host, Outputs, Request, Runner, Snapshot};
use crate::{theme::ThemedScrollArea, toolkits::Action};
use eframe::egui;
use matrust::{
    Array, StructArray, TableArray,
    plot::{Axes, ImageMapping, ImageObject},
};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

pub(crate) type OpenSlot = Arc<Mutex<Option<Option<PathBuf>>>>;
const DISPLAY_SIDE: usize = 512;
const MAX_SERIES: usize = 100_000;
const MAX_VALUES: usize = 16_777_216;
const MAX_METADATA_FIELDS: usize = 100_000;
const MAX_METADATA_BYTES: usize = 16 * 1024 * 1024;
const SERIES_HEADERS: [&str; 6] = [
    "Study date",
    "Patient name",
    "Modality",
    "Series description",
    "Rows × cols",
    "Frames",
];
const SERIES_NUMERIC: [bool; 6] = [false, false, false, false, true, true];
/// `string(..., 'yyyy-MM-dd HH:mm:ss')` is this long. The date column keeps it.
const DATE_SAMPLE: &str = "2024-01-15 09:30:00";
const EMPTY_DATE: &str = "—";
const ROW_H: f32 = 24.;
const SORT_GUTTER: f32 = 12.;
const CELL_PAD: f32 = 4.;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spec {
    pub folder: String,
    pub recursive: bool,
    /// True only for the collection produced by Use sample data. A later Scan
    /// clears it, so the shown folder is read like any other folder.
    pub sample: bool,
}
impl Default for Spec {
    fn default() -> Self {
        Self {
            folder: String::new(),
            recursive: true,
            sample: false,
        }
    }
}
impl Spec {
    fn resolved(&self, ws: &Snapshot) -> Result<Self, String> {
        super::comparison_tool::path_literal(&self.folder)?;
        let path = PathBuf::from(&self.folder);
        let path = if path.is_relative() {
            ws.folder().map_or(path.clone(), |p| p.join(&path))
        } else {
            path
        };
        let folder = path
            .to_str()
            .ok_or("Folder must be valid Unicode")?
            .to_owned();
        super::comparison_tool::path_literal(&folder)?;
        Ok(Self {
            folder,
            recursive: self.recursive,
            sample: false,
        })
    }
}

const RESERVED_FUNCTIONS: [&str; 4] = [
    "matrust_dicom_series",
    "matrust_dicom_scan",
    "matrust_dicom_sample",
    "matrust_dicom_clear",
];

/// Output names are assignment targets, never expressions or engine builtin names.
pub fn checked_names(names: &[String; 3]) -> Result<(), String> {
    for (i, name) in names.iter().enumerate() {
        if name.len() > 63 || !matrust::files::valid_name(name) {
            return Err("Use variable names of at most 63 characters".into());
        }
        super::check_export_name(name)?;
        if matrust::session::FUNCTIONS.contains(&name.as_str())
            || RESERVED_FUNCTIONS.contains(&name.as_str())
        {
            return Err(format!("{name} is a function name; choose another name"));
        }
        matrust::syntax::parse(&format!("{name} = [];"))?;
        if names[..i].contains(name) {
            return Err("Volume, metadata and collection need distinct names".into());
        }
    }
    Ok(())
}

/// None scans only. Some selects by stable UID, reads every file in collection
/// order and concatenates along dicomread's fourth (frame) dimension. A local
/// function isolates helpers from workspace names and transactional rollback.
/// Use sample data is the same function with `sample` true, so Generate script
/// writes the folder again before scanning it.
pub fn build_script(spec: &Spec, uid: Option<&str>, names: &[String; 3]) -> Result<String, String> {
    checked_names(names)?;
    let folder = if spec.sample {
        "''".to_owned()
    } else {
        super::comparison_tool::path_literal(&spec.folder)?
    };
    let recursive = if spec.recursive { "true" } else { "false" };
    let sample = if spec.sample { "true" } else { "false" };
    let [volume, info, collection] = names;
    let mut code = if let Some(uid) = uid {
        let uid = super::comparison_tool::path_literal(uid)?;
        format!(
            "[{volume}, {info}, {collection}] = matrust_dicom_series({folder}, {recursive}, {uid}, {sample});\n\
             function [volume, metadata, collection] = matrust_dicom_series(folder, recursive, uid, sample)\n\
             [collection, folder] = matrust_dicom_scan(folder, recursive, sample);\n\
             row = find(collection.SeriesInstanceUID == string(uid));\n\
             if numel(row) ~= 1, error('Selected series is no longer in the collection'); end\n\
             files = collection.Filenames{{row}};\n\
             if isempty(files), error('Selected series has no files'); end\n\
             if collection.Rows(row)*collection.Columns(row)*collection.Channels(row)*collection.Frames(row) > {MAX_VALUES}, error('Selected series exceeds the browser volume limit'); end\n\
             for k = 1:numel(files)\n\
               part = dicomread(files(k));\n\
               part = reshape(part, size(part,1), size(part,2), size(part,3), size(part,4));\n\
               next = dicominfo(files(k));\n\
               if k == 1\n\
                 volume = part; metadata = next;\n\
               else\n\
                 volume = cat(4, volume, part);\n\
                 fields = fieldnames(metadata);\n\
                 for j = 1:numel(fields)\n\
                   field = fields{{j}};\n\
                   if ~isfield(next, field), next.(field) = []; end\n\
                 end\n\
                 fields = fieldnames(next);\n\
                 for j = 1:numel(fields)\n\
                   field = fields{{j}};\n\
                   if ~isfield(metadata, field), metadata(1).(field) = []; end\n\
                 end\n\
                 metadata(k) = next;\n\
               end\n\
             end\n\
             end\n"
        )
    } else {
        format!(
            "[{collection}, db_folder] = matrust_dicom_scan({folder}, {recursive}, {sample});\n"
        )
    };
    code.push_str(SAMPLE_HELPERS);
    Ok(code)
}

/// Copy mode keeps RescaleIntercept, SliceLocation and ImagePositionPatient;
/// create mode drops them and would invent UIDs. Fixed UIDs keep two runs equal.
const SAMPLE_HELPERS: &str = r#"
function [collection, folder] = matrust_dicom_scan(folder, recursive, sample)
if sample
  folder = matrust_dicom_sample();
end
collection = dicomCollection(folder, 'IncludeSubfolders', recursive);
end
function folder = matrust_dicom_sample()
folder = fullfile(tempdir, 'matrust-dicom-sample');
if ~isdir(folder)
  mkdir(folder);
end
for k = 1:20
  matrust_dicom_clear(fullfile(folder, ['head_' num2str(k) '.dcm']));
end
matrust_dicom_clear(fullfile(folder, 'scout.dcm'));
matrust_dicom_clear(fullfile(folder, 'knee.dcm'));
matrust_dicom_clear(fullfile(folder, 'damaged.dcm'));
[xg, yg] = meshgrid(1:128, 1:128);
for k = 1:20
  t = (k - 1) / 19;
  rx = 36 + 16 * t;
  ry = 48 - 10 * t;
  outer = (((xg - 64.5) ./ rx) .^ 2) + (((yg - 64.5) ./ ry) .^ 2) <= 1;
  inner = (((xg - 64.5) ./ (rx - 5)) .^ 2) + (((yg - 64.5) ./ (ry - 5)) .^ 2) <= 1;
  vx = 8 + 6 * t;
  vr = 3 + 4 * t;
  left = (((xg - (64.5 - vx)) ./ vr) .^ 2) + (((yg - (58 + 6 * t)) ./ (vr + 2)) .^ 2) <= 1;
  right = (((xg - (64.5 + vx)) ./ (vr + 1)) .^ 2) + (((yg - (72 - 4 * t)) ./ vr) .^ 2) <= 1;
  img = zeros(128, 128);
  img(outer) = 1900;
  img(inner) = 1080;
  img(left) = 760;
  img(right) = 760;
  z = -47.5 + (k - 1) * 5;
  meta = struct('PatientName', 'Sample^Head', 'Modality', 'CT');
  meta.StudyDate = '20240115';
  meta.StudyTime = '090000';
  meta.SeriesDate = meta.StudyDate;
  meta.SeriesTime = '090000';
  meta.SeriesDescription = 'Head phantom';
  meta.SOPClassUID = '1.2.840.10008.5.1.4.1.1.2';
  meta.StudyInstanceUID = '1.2.826.0.1.3680043.10.543.1';
  meta.SeriesInstanceUID = '1.2.826.0.1.3680043.10.543.1.1';
  meta.RescaleIntercept = -1024;
  meta.RescaleSlope = 1;
  meta.InstanceNumber = k;
  meta.SliceLocation = z;
  meta.ImagePositionPatient = [0; 0; z];
  meta.SOPInstanceUID = ['1.2.826.0.1.3680043.10.543.1.1.' num2str(k)];
  dicomwrite(uint16(img), fullfile(folder, ['head_' num2str(k) '.dcm']), meta, 'CreateMode', 'copy');
end
img = zeros(128, 128);
img(abs(xg - 64.5) <= 4) = 1600;
img(abs(yg - 64.5) <= 4) = 1600;
img(((xg - 40) .^ 2) + ((yg - 90) .^ 2) <= 144) = 400;
meta = struct('PatientName', 'Sample^Head', 'Modality', 'CT');
meta.StudyDate = '20240115';
meta.StudyTime = '100000';
meta.SeriesDate = meta.StudyDate;
meta.SeriesTime = '100000';
meta.SeriesDescription = 'Scout';
meta.SOPClassUID = '1.2.840.10008.5.1.4.1.1.2';
meta.StudyInstanceUID = '1.2.826.0.1.3680043.10.543.2';
meta.SeriesInstanceUID = '1.2.826.0.1.3680043.10.543.2.1';
meta.RescaleIntercept = -1024;
meta.RescaleSlope = 1;
meta.InstanceNumber = 1;
meta.SliceLocation = 0;
meta.ImagePositionPatient = [0; 0; 0];
meta.SOPInstanceUID = '1.2.826.0.1.3680043.10.543.2.1.1';
dicomwrite(uint16(img), fullfile(folder, 'scout.dcm'), meta, 'CreateMode', 'copy');
img = zeros(128, 128);
img((((xg - 48) ./ 18) .^ 2) + (((yg - 40) ./ 28) .^ 2) <= 1) = 1400;
img((((xg - 80) ./ 16) .^ 2) + (((yg - 88) ./ 26) .^ 2) <= 1) = 1400;
img(abs(xg - 64) <= 3) = 200;
meta = struct('PatientName', 'Sample^Knee', 'Modality', 'MR');
meta.StudyDate = '20240302';
meta.StudyTime = '110000';
meta.SeriesDate = meta.StudyDate;
meta.SeriesTime = '110000';
meta.SeriesDescription = 'Knee MR';
meta.SOPClassUID = '1.2.840.10008.5.1.4.1.1.4';
meta.StudyInstanceUID = '1.2.826.0.1.3680043.10.543.3';
meta.SeriesInstanceUID = '1.2.826.0.1.3680043.10.543.3.1';
meta.InstanceNumber = 1;
meta.SOPInstanceUID = '1.2.826.0.1.3680043.10.543.3.1.1';
dicomwrite(uint16(img), fullfile(folder, 'knee.dcm'), meta, 'CreateMode', 'copy');
fid = fopen(fullfile(folder, 'damaged.dcm'), 'w');
if fid < 0
  error('Could not write the damaged DICOM sample');
end
fwrite(fid, zeros(1, 128), 'uint8');
fwrite(fid, 'DICM', 'uint8');
fwrite(fid, 'truncated-dicom-sample', 'uint8');
fwrite(fid, zeros(1, 178), 'uint8');
fclose(fid);
end
function matrust_dicom_clear(path)
if exist(path, 'file') == 2
  delete(path);
end
end
"#;
fn private_names() -> [String; 3] {
    ["db_volume", "db_info", "db_collection"].map(str::to_owned)
}

#[derive(Clone, Debug)]
pub struct Series {
    pub uid: String,
    pub cells: [String; 6],
    pub rows: usize,
    pub cols: usize,
    pub frames: usize,
}
pub struct Collection {
    pub spec: Spec,
    pub table: TableArray,
    pub rows: Vec<Series>,
    /// Regular files the scan walked, minus files named in `Filenames`.
    pub skipped: Result<usize, String>,
}
pub struct Preview {
    pub uid: String,
    pub volume: Array,
    pub info: StructArray,
    pub metadata: Vec<Vec<(String, String)>>,
    pub frame_info: Vec<usize>,
    pub frames: usize,
    pub mono16: bool,
}
#[derive(Clone)]
enum Job {
    Scan(Spec),
    Load(Spec, String),
    Frame(usize),
}
enum Prepared {
    Scan(Collection),
    Load(Arc<Preview>, Box<Axes>, [f64; 2]),
    Frame(usize, Box<Axes>, [f64; 2]),
}

pub struct DicomBrowser {
    pub draft: Spec,
    pub collection: Option<Arc<Collection>>,
    pub current: Option<Arc<Preview>>,
    pub selected: Option<String>,
    pub frame: usize,
    pub shown_frame: usize,
    pub window: f64,
    pub level: f64,
    range: [f64; 2],
    pub filter: String,
    pub names: [String; 3],
    pub inline_error: Option<String>,
    pub axes: Option<Axes>,
    pub laid_out_rows: usize,
    order: Vec<usize>,
    sort: (usize, bool),
    job: Option<Job>,
    scan_requested: bool,
    sample_requested: bool,
    select_requested: Option<String>,
    pub(crate) frame_requested: bool,
    stop_requested: bool,
    pick_requested: bool,
    running: bool,
    serial: u64,
    payload: Arc<Mutex<Option<Prepared>>>,
    input_error: Option<String>,
    #[cfg(test)]
    pub controls: std::collections::BTreeMap<String, egui::Rect>,
}
impl Default for DicomBrowser {
    fn default() -> Self {
        Self {
            draft: Spec::default(),
            collection: None,
            current: None,
            selected: None,
            frame: 0,
            shown_frame: 0,
            window: 1.,
            level: 0.5,
            range: [0., 1.],
            filter: String::new(),
            names: ["dicomVolume", "dicomInfo", "dicomSeries"].map(str::to_owned),
            inline_error: None,
            axes: None,
            laid_out_rows: 0,
            order: vec![],
            sort: (0, false),
            job: None,
            scan_requested: false,
            sample_requested: false,
            select_requested: None,
            frame_requested: false,
            stop_requested: false,
            pick_requested: false,
            running: false,
            serial: 0,
            payload: Default::default(),
            input_error: None,
            #[cfg(test)]
            controls: Default::default(),
        }
    }
}
impl DicomBrowser {
    pub fn scan(&mut self) {
        self.scan_requested = true;
    }
    pub fn use_sample(&mut self) {
        self.sample_requested = true;
    }
    pub fn select(&mut self, uid: String) {
        self.select_requested = Some(uid);
    }
    pub fn choose(&mut self, folder: PathBuf) {
        self.draft.folder = folder.to_string_lossy().into_owned();
        self.scan();
    }
    fn record(&mut self, name: impl Into<String>, response: &egui::Response) {
        #[cfg(test)]
        self.controls.insert(name.into(), response.rect);
        #[cfg(not(test))]
        let _ = (name, response);
    }
    pub fn sort_by(&mut self, column: usize) {
        if self.sort.0 == column {
            self.sort.1 = !self.sort.1;
        } else {
            self.sort = (column, false);
        }
        self.sort_rows();
    }
    fn sort_rows(&mut self) {
        let Some(collection) = &self.collection else {
            return;
        };
        self.order = (0..collection.rows.len()).collect();
        let (column, descending) = self.sort;
        self.order.sort_by(|&a, &b| {
            let (a, b) = (&collection.rows[a], &collection.rows[b]);
            let order = match column {
                0 => date_key(&a.cells[0]).cmp(date_key(&b.cells[0])),
                4 => (a.rows, a.cols).cmp(&(b.rows, b.cols)),
                5 => a.frames.cmp(&b.frames),
                _ => a.cells[column].cmp(&b.cells[column]),
            }
            .then_with(|| a.uid.cmp(&b.uid));
            if descending { order.reverse() } else { order }
        });
    }
    fn selected_script(&self) -> Result<(&Spec, &str), String> {
        let collection = self
            .collection
            .as_ref()
            .ok_or("Choose a folder and press Scan")?;
        let uid = self.selected.as_deref().ok_or("Select a series")?;
        if self.current.as_ref().is_none_or(|p| p.uid != uid) {
            return Err("Read the selected series successfully before exporting".into());
        }
        Ok((&collection.spec, uid))
    }

    /// Flat series table. Columns share the main area: description gives up
    /// width first, then the patient name. Dates and the numeric headers keep
    /// their measured width until nothing else can shrink.
    fn series_table(&mut self, ui: &mut egui::Ui, max_height: f32) {
        let Some(collection) = self.collection.clone() else {
            return;
        };
        let budget = (ui.available_width() - ui.spacing().scroll.bar_width - 1.).max(160.);
        let widths = series_widths(ui, budget);
        let table_w: f32 = widths.iter().sum();
        let mut sort = None;
        self.header_row(ui, &widths, table_w, &mut sort);
        egui::ScrollArea::vertical()
            .id_salt("dicom_series_y")
            .max_height(max_height)
            .show_rows_themed(ui, ROW_H, self.order.len(), |ui, range| {
                for index in range {
                    self.laid_out_rows += 1;
                    self.series_row(ui, &collection, index, &widths, table_w);
                }
            });
        if let Some(column) = sort {
            self.sort_by(column);
        }
    }

    fn header_row(
        &mut self,
        ui: &mut egui::Ui,
        widths: &[f32; 6],
        table_w: f32,
        sort: &mut Option<usize>,
    ) {
        let p = crate::tokens::palette(ui.ctx());
        let (rect, _) = ui.allocate_exact_size(egui::vec2(table_w, ROW_H), egui::Sense::hover());
        ui.painter().hline(
            rect.x_range(),
            rect.bottom() - 1.,
            egui::Stroke::new(1., p.hairline),
        );
        let mut row = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(false))
                .id_salt("dicom_header"),
        );
        row.spacing_mut().item_spacing.x = 0.;
        for column in 0..6 {
            let (cell, response) =
                row.allocate_exact_size(egui::vec2(widths[column], ROW_H), egui::Sense::click());
            let elided = paint_text(
                &row,
                cell,
                SERIES_HEADERS[column],
                SERIES_NUMERIC[column],
                SORT_GUTTER,
            );
            if self.sort.0 == column {
                let used = text_width(&row, SERIES_HEADERS[column])
                    .min((cell.width() - CELL_PAD - SORT_GUTTER).max(0.));
                // Text columns are often wider than their title. Keep the mark
                // beside that title; numeric titles already end at the gutter.
                let x = if SERIES_NUMERIC[column] {
                    cell.right() - SORT_GUTTER * 0.5
                } else {
                    (cell.left() + CELL_PAD + used + 8.).min(cell.right() - 6.)
                };
                sort_mark(
                    row.painter(),
                    egui::pos2(x, cell.center().y),
                    self.sort.1,
                    p.accent,
                );
            }
            let response = if elided {
                response.on_hover_text(SERIES_HEADERS[column])
            } else {
                response
            };
            self.record(format!("sort{column}"), &response);
            if response.clicked() {
                *sort = Some(column);
            }
        }
    }

    fn series_row(
        &mut self,
        ui: &mut egui::Ui,
        collection: &Collection,
        index: usize,
        widths: &[f32; 6],
        table_w: f32,
    ) {
        let series = &collection.rows[self.order[index]];
        let uid = series.uid.clone();
        let cells = series.cells.clone();
        let selected = self.selected.as_deref() == Some(uid.as_str());
        let (rect, _) = ui.allocate_exact_size(egui::vec2(table_w, ROW_H), egui::Sense::hover());
        if selected {
            ui.painter()
                .rect_filled(rect, 0., crate::tokens::palette(ui.ctx()).accent_soft);
        }
        let mut row = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(false))
                .id_salt(("dicom_row", index)),
        );
        row.spacing_mut().item_spacing.x = 0.;
        for (column, text) in cells.iter().enumerate() {
            let (cell, response) =
                row.allocate_exact_size(egui::vec2(widths[column], ROW_H), egui::Sense::click());
            let elided = paint_text(&row, cell, text, SERIES_NUMERIC[column], SORT_GUTTER);
            let response = if elided {
                response.on_hover_text(text)
            } else {
                response
            };
            self.record(format!("row{uid}_{column}"), &response);
            if response.clicked() {
                self.select(uid.clone());
            }
        }
    }

    /// Label, then a painted track (the theme slider rail matches the panel),
    /// then `k / N`. Hidden for a single frame. The recorded rect is the track.
    /// With focus, Left/Right step one frame and Home/End jump to the ends.
    fn frame_control(&mut self, ui: &mut egui::Ui, frames: usize) {
        if frames <= 1 {
            return;
        }
        let enabled = !self.running
            && self
                .current
                .as_ref()
                .is_some_and(|preview| self.selected.as_deref() == Some(preview.uid.as_str()));
        let p = crate::tokens::palette(ui.ctx());
        let readout = format!("{} / {frames}", self.frame + 1);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.;
            let label_w = text_width(ui, "Frame") + 12.;
            let readout_w = text_width(ui, &format!("{frames} / {frames}")) + 12.;
            let (label_rect, _) =
                ui.allocate_exact_size(egui::vec2(label_w, ROW_H), egui::Sense::hover());
            paint_text(ui, label_rect, "Frame", false, CELL_PAD);
            let room = (ui.available_width() - readout_w - 8.).max(48.);
            let slider_w = if room >= 160. { room.min(240.) } else { room };
            let sense = if enabled {
                egui::Sense::click_and_drag()
            } else {
                egui::Sense::hover()
            };
            let (rect, response) = ui.allocate_exact_size(egui::vec2(slider_w, ROW_H), sense);
            let radius = 8.;
            let track = egui::Rect::from_min_max(
                egui::pos2(rect.left() + radius, rect.center().y - 4.),
                egui::pos2(rect.right() - radius, rect.center().y + 4.),
            );
            ui.painter().rect_filled(track, 4., p.border);
            let span = (frames - 1) as f32;
            let t = if span == 0. {
                0.
            } else {
                self.frame as f32 / span
            };
            let handle_x = track.left() + t * track.width();
            ui.painter().rect_filled(
                egui::Rect::from_min_max(track.left_top(), egui::pos2(handle_x, track.bottom())),
                4.,
                p.accent,
            );
            let handle = egui::pos2(handle_x, rect.center().y);
            ui.painter().circle_filled(handle, radius, p.surface);
            ui.painter()
                .circle_stroke(handle, radius, egui::Stroke::new(2., p.accent));
            if enabled
                && (response.dragged() || response.clicked())
                && let Some(pos) = response.interact_pointer_pos()
            {
                let at = ((pos.x - track.left()) / track.width().max(1.)).clamp(0., 1.);
                let next = ((at * span).round() as usize).min(frames - 1);
                if next != self.frame {
                    self.frame = next;
                    self.frame_requested = true;
                }
            }
            if enabled && (response.clicked() || response.drag_started()) {
                response.request_focus();
            }
            if enabled && response.has_focus() {
                // Keep Left/Right on this track, as `egui::Slider` does.
                ui.memory_mut(|mem| {
                    mem.set_focus_lock_filter(
                        response.id,
                        egui::EventFilter {
                            horizontal_arrows: true,
                            ..Default::default()
                        },
                    );
                });
                let next = ui.input(|input| {
                    if input.key_pressed(egui::Key::Home) {
                        Some(0)
                    } else if input.key_pressed(egui::Key::End) {
                        Some(frames - 1)
                    } else {
                        let step = input.num_presses(egui::Key::ArrowRight) as isize
                            - input.num_presses(egui::Key::ArrowLeft) as isize;
                        (step != 0)
                            .then_some((self.frame as isize + step).clamp(0, (frames - 1) as isize)
                                as usize)
                    }
                });
                if let Some(next) = next
                    && next != self.frame
                {
                    self.frame = next;
                    self.frame_requested = true;
                }
            }
            self.record("Frame", &response);
            let (read_rect, _) =
                ui.allocate_exact_size(egui::vec2(readout_w, ROW_H), egui::Sense::hover());
            paint_text(ui, read_rect, &readout, false, CELL_PAD);
            crate::panels::focus_ring(ui, &response);
        });
    }
}

impl App for DicomBrowser {
    fn validate_idle_inputs(&self, ws: &Snapshot) -> Result<(), String> {
        checked_names(&self.names)?;
        if !self.draft.folder.is_empty() {
            self.draft.resolved(ws)?;
        }
        Ok(())
    }
    fn idle_hint(&self) -> Option<&'static str> {
        (self.job.is_none() && self.input_error.is_none())
            .then_some("Choose a folder and press Scan")
    }
    fn kind(&self) -> AppKind {
        AppKind::DicomBrowser
    }
    fn script(&self, _ws: &Snapshot) -> Result<String, String> {
        let (spec, uid) = self.selected_script()?;
        build_script(spec, Some(uid), &self.names)
    }
    fn export_code(&self, ws: &Snapshot) -> Result<String, String> {
        self.script(ws)
    }
    fn execution_script(&self, _ws: &Snapshot) -> Result<String, String> {
        if let Some(error) = &self.input_error {
            return Err(error.clone());
        }
        match &self.job {
            Some(Job::Scan(spec)) => build_script(spec, None, &private_names()),
            Some(Job::Load(spec, uid)) => build_script(spec, Some(uid), &private_names()),
            Some(Job::Frame(frame)) => Ok(format!("db_frame = db_volume(:,:,:,{});\n", frame + 1)),
            None => Err("Choose a folder and press Scan".into()),
        }
    }
    fn inputs(&self) -> Vec<String> {
        vec![]
    }
    fn prelude(&self, _ws: &Snapshot) -> String {
        format!("% DICOM request {}\n", self.serial)
    }
    fn probe(&self, _ws: &Snapshot) -> (String, Vec<String>) {
        match &self.job {
            Some(Job::Scan(_)) => (
                "db_dates = string(db_collection.StudyDateTime, 'yyyy-MM-dd HH:mm:ss');\n".into(),
                vec![
                    "db_collection".into(),
                    "db_dates".into(),
                    "db_folder".into(),
                ],
            ),
            Some(Job::Load(..)) => (String::new(), private_names().to_vec()),
            Some(Job::Frame(_)) => (String::new(), vec!["db_frame".into()]),
            None => (String::new(), vec![]),
        }
    }
    fn prepare(&mut self, ws: &Snapshot) {
        if std::mem::take(&mut self.sample_requested) {
            self.scan_requested = false;
            self.serial = self.serial.wrapping_add(1);
            self.select_requested = None;
            self.frame_requested = false;
            self.job = Some(Job::Scan(Spec {
                folder: self.draft.folder.clone(),
                recursive: self.draft.recursive,
                sample: true,
            }));
            self.input_error = None;
        } else if std::mem::take(&mut self.scan_requested) {
            self.serial = self.serial.wrapping_add(1);
            self.select_requested = None;
            self.frame_requested = false;
            match self.draft.resolved(ws) {
                Ok(spec) => {
                    self.job = Some(Job::Scan(spec));
                    self.input_error = None;
                }
                Err(error) => {
                    self.job = None;
                    self.input_error = Some(error);
                }
            }
        } else if let Some(uid) = self.select_requested.take() {
            if let Some(collection) = &self.collection {
                self.serial = self.serial.wrapping_add(1);
                self.selected = Some(uid.clone());
                self.inline_error = None;
                self.input_error = None;
                self.job = Some(Job::Load(collection.spec.clone(), uid));
                self.frame_requested = false;
            }
        } else if std::mem::take(&mut self.frame_requested) {
            self.serial = self.serial.wrapping_add(1);
            self.job = Some(Job::Frame(self.frame));
            self.input_error = None;
        }
    }
    fn submit(&self, runner: &mut Runner, ctx: &egui::Context, request: Request) {
        if runner.key() == Some(request.key) {
            runner.poll(ctx);
            return;
        }
        let Some(job) = self.job.clone() else {
            return;
        };
        let current = self.current.clone();
        let payload = self.payload.clone();
        runner.submit_task(
            ctx,
            request.key,
            Box::new(move |engine| {
                if matches!(job, Job::Frame(_)) {
                    engine.insert_array(
                        "db_volume",
                        current.as_ref().ok_or("No preview")?.volume.clone(),
                    )?;
                }
                let (outputs, skipped) =
                    matrust::engine::with_dicom_collection_count(|| super::run(engine, request));
                let outputs = outputs?;
                let cancel = engine.cancellation();
                let prepared = match job {
                    Job::Scan(spec) => {
                        Prepared::Scan(read_collection(spec, &outputs, skipped, &cancel)?)
                    }
                    Job::Load(_, uid) => {
                        let preview = Arc::new(read_preview(uid, &outputs, &cancel)?);
                        let (axes, range) = frame_axes(&preview.volume, 0, &cancel)?;
                        Prepared::Load(preview, Box::new(axes), range)
                    }
                    Job::Frame(frame) => {
                        let (axes, range) =
                            frame_axes(outputs.get("db_frame").ok_or("No frame")?, 0, &cancel)?;
                        Prepared::Frame(frame, Box::new(axes), range)
                    }
                };
                if cancel.load(Ordering::Relaxed) {
                    return Err("DICOM operation cancelled".into());
                }
                *payload.lock().map_err(|_| "DICOM result lock failed")? = Some(prepared);
                Ok(outputs)
            }),
        );
    }
    fn receive(&mut self, _outputs: &Outputs) -> Result<(), String> {
        let prepared = self
            .payload
            .lock()
            .map_err(|_| "DICOM result lock failed")?
            .take()
            .ok_or("No DICOM result")?;
        match prepared {
            Prepared::Scan(collection) => {
                if collection.spec.sample {
                    self.draft.folder = collection.spec.folder.clone();
                }
                self.collection = Some(Arc::new(collection));
                self.selected = None;
                self.sort_rows();
                self.inline_error = None;
            }
            Prepared::Load(preview, axes, range) => {
                self.current = Some(preview);
                self.frame = 0;
                self.shown_frame = 0;
                self.axes = Some(*axes);
                self.window = (range[1] - range[0]).max(1.);
                self.level = (range[0] + range[1]) / 2.;
                self.range = range;
                self.inline_error = None;
            }
            Prepared::Frame(frame, axes, range) => {
                self.shown_frame = frame;
                self.axes = Some(*axes);
                self.range = range;
            }
        }
        Ok(())
    }
    fn inputs_ui(&mut self, ui: &mut egui::Ui, _ws: &Snapshot) {
        super::field(ui, "Folder", |ui| {
            ui.add(egui::TextEdit::singleline(&mut self.draft.folder).desired_width(f32::INFINITY));
        });
        let browse = ui.button("Browse…");
        self.record("Browse", &browse);
        if browse.clicked() {
            self.pick_requested = true;
        }
        ui.add_space(8.);
        ui.checkbox(&mut self.draft.recursive, "Include subfolders");
        ui.add_space(8.);
        ui.horizontal_wrapped(|ui| {
            let scan = ui.add_enabled(!self.running, egui::Button::new("Scan"));
            self.record("Scan", &scan);
            if scan.clicked() {
                self.scan();
            }
            let sample = ui.add_enabled(!self.running, egui::Button::new("Use sample data"));
            self.record("Use sample data", &sample);
            if sample.clicked() {
                self.use_sample();
            }
            let cancel = ui.add_enabled(self.running, egui::Button::new("Cancel"));
            self.record("Cancel", &cancel);
            if cancel.clicked() {
                self.stop_requested = true;
            }
        });
        ui.add_space(8.);
        crate::tokens::helper(
            ui,
            if self.running {
                "Reading DICOM files…".into()
            } else {
                self.summary()
                    .unwrap_or_else(|| "No collection scanned".into())
            },
        );
    }
    fn main_caption(&self) -> &'static str {
        "SERIES AND PREVIEW"
    }
    fn main_ui(&mut self, ui: &mut egui::Ui) {
        self.laid_out_rows = 0;
        if self.collection.is_some() {
            self.series_table(ui, 176.);
        } else {
            crate::tokens::helper(ui, "Scan a folder to list its image series.");
        }
        ui.add_space(8.);
        if let Some(label) = self
            .collection
            .as_ref()
            .and_then(|c| skipped_label(&c.skipped))
        {
            ui.label(label);
        }
        if let Some(error) = &self.inline_error {
            ui.colored_label(crate::tokens::palette(ui.ctx()).text, error);
        }
        let Some(current) = self.current.clone() else {
            return;
        };
        if self.selected.as_deref() != Some(&current.uid) {
            crate::tokens::helper(ui, "Last successful preview");
        }
        self.frame_control(ui, current.frames);
        if current.mono16 {
            ui.horizontal_wrapped(|ui| {
                ui.label("Window");
                ui.add(egui::DragValue::new(&mut self.window).range(1.0..=131072.0));
                ui.label("Level");
                ui.add(egui::DragValue::new(&mut self.level).range(-65536.0..=65536.0));
                if ui.button("Min / max").clicked() {
                    self.window = (self.range[1] - self.range[0]).max(1.);
                    self.level = (self.range[0] + self.range[1]) / 2.;
                }
            });
        }
        if let Some(axes) = &mut self.axes {
            axes.extras.clim = Some([self.level - self.window / 2., self.level + self.window / 2.]);
            let plot_id = egui::Id::new("dicom_preview");
            egui_plot::Plot::new(plot_id)
                .height(192.)
                .data_aspect(1.)
                .show_axes(false)
                .show_grid(false)
                .show(ui, |plot| {
                    crate::figure_images::add(plot, axes, plot_id);
                });
        }
        ui.add_space(8.);
        ui.horizontal(|ui| {
            ui.label("Metadata filter");
            ui.text_edit_singleline(&mut self.filter);
        });
        let filter = self.filter.to_lowercase();
        let info_index = current.frame_info[self.shown_frame];
        let fields = &current.metadata[info_index];
        let visible: Vec<_> = fields
            .iter()
            .enumerate()
            .filter_map(|(i, (name, value))| {
                (filter.is_empty()
                    || name.to_lowercase().contains(&filter)
                    || value.to_lowercase().contains(&filter))
                .then_some(i)
            })
            .collect();
        let name_width = (ui.available_width() * 0.4).clamp(72., 224.);
        metadata_table(ui, name_width, fields, &visible, 160.);
    }

    fn results(&self) -> Vec<(&'static str, String)> {
        vec![
            (
                "Series",
                self.collection
                    .as_ref()
                    .map_or(0, |c| c.table.height())
                    .to_string(),
            ),
            (
                "Frames",
                self.current.as_ref().map_or(0, |p| p.frames).to_string(),
            ),
            (
                "Files",
                self.current
                    .as_ref()
                    .map_or(0, |p| p.info.len())
                    .to_string(),
            ),
        ]
    }
    fn names_ui(&mut self, ui: &mut egui::Ui, _ws: &Snapshot) {
        for (label, name) in ["Volume", "Metadata", "Collection"]
            .into_iter()
            .zip(&mut self.names)
        {
            super::field(ui, label, |ui| {
                ui.text_edit_singleline(name);
            });
        }
        if let Err(error) = checked_names(&self.names) {
            crate::tokens::helper(ui, error);
        }
    }
    fn exported(&self, _ws: &Snapshot) -> Vec<String> {
        self.names.to_vec()
    }
    fn summary(&self) -> Option<String> {
        self.collection.as_ref().map(|c| {
            format!(
                "{} series · {} frames",
                c.rows.len(),
                c.rows
                    .iter()
                    .fold(0usize, |n, r| n.saturating_add(r.frames))
            )
        })
    }
}

fn date_key(text: &str) -> &str {
    if text == EMPTY_DATE { "" } else { text }
}

fn text_width(ui: &egui::Ui, text: &str) -> f32 {
    ui.painter()
        .layout_no_wrap(
            text.to_owned(),
            egui::TextStyle::Body.resolve(ui.style()),
            egui::Color32::PLACEHOLDER,
        )
        .size()
        .x
}

/// Measured text plus the left pad, the sort gutter and 4 px so the sample
/// itself is not elided.
fn column_width(text_w: f32) -> f32 {
    text_w + CELL_PAD + SORT_GUTTER + 4.
}

fn series_widths(ui: &egui::Ui, budget: f32) -> [f32; 6] {
    let samples = [
        DATE_SAMPLE,
        "SMITH^BOB",
        "Modality",
        "Series description",
        "Rows × cols",
        "Frames",
    ];
    let mut widths = [0f32; 6];
    for i in 0..6 {
        widths[i] = column_width(text_width(ui, SERIES_HEADERS[i]).max(text_width(ui, samples[i])));
    }
    let sum: f32 = widths.iter().sum();
    if sum <= budget {
        widths[3] += budget - sum;
        return widths;
    }
    let mut over = sum - budget;
    let floors = [
        column_width(text_width(ui, DATE_SAMPLE)),
        column_width(48.),
        column_width(text_width(ui, "CT").max(24.)),
        column_width(32.),
        column_width(text_width(ui, "Rows × cols")),
        column_width(text_width(ui, "Frames")),
    ];
    for index in [3, 1, 2, 4, 5, 0] {
        if over <= 0. {
            break;
        }
        let take = (widths[index] - floors[index]).max(0.).min(over);
        widths[index] -= take;
        over -= take;
    }
    if over > 0. {
        let sum: f32 = widths.iter().sum();
        let scale = budget / sum;
        for width in &mut widths {
            *width *= scale;
        }
    }
    widths
}

/// Returns whether the text was elided. Numeric columns end at the sort gutter
/// so the indicator does not shift the digits.
fn paint_text(
    ui: &egui::Ui,
    rect: egui::Rect,
    text: &str,
    align_right: bool,
    right_inset: f32,
) -> bool {
    let color = crate::tokens::palette(ui.ctx()).text;
    let left = rect.left() + CELL_PAD;
    let right = rect.right() - right_inset;
    let max_w = (right - left).max(0.);
    let mut job = egui::text::LayoutJob::default();
    job.append(
        text,
        0.,
        egui::TextFormat {
            font_id: egui::TextStyle::Body.resolve(ui.style()),
            color,
            ..Default::default()
        },
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_w);
    let galley = ui.painter().layout_job(job);
    let y = rect.center().y - galley.size().y * 0.5;
    let x = if align_right {
        right - galley.size().x
    } else {
        left
    };
    ui.painter()
        .with_clip_rect(rect)
        .galley(egui::pos2(x, y), galley.clone(), color);
    galley.elided
}

fn sort_mark(painter: &egui::Painter, center: egui::Pos2, descending: bool, color: egui::Color32) {
    let (base, tip) = if descending {
        (center.y - 3., center.y + 3.)
    } else {
        (center.y + 3., center.y - 3.)
    };
    painter.add(egui::Shape::convex_polygon(
        vec![
            egui::pos2(center.x - 4., base),
            egui::pos2(center.x + 4., base),
            egui::pos2(center.x, tip),
        ],
        color,
        egui::Stroke::NONE,
    ));
}

fn metadata_table(
    ui: &mut egui::Ui,
    name_width: f32,
    fields: &[(String, String)],
    visible: &[usize],
    max_height: f32,
) {
    let paint_pair = |ui: &mut egui::Ui, name: &str, value: &str| {
        ui.spacing_mut().item_spacing.x = 8.;
        ui.horizontal(|ui| {
            let (name_rect, name_response) =
                ui.allocate_exact_size(egui::vec2(name_width, ROW_H), egui::Sense::hover());
            if paint_text(ui, name_rect, name, false, CELL_PAD) {
                name_response.on_hover_text(name);
            }
            let value_w = ui.available_width();
            let (value_rect, value_response) =
                ui.allocate_exact_size(egui::vec2(value_w, ROW_H), egui::Sense::hover());
            if paint_text(ui, value_rect, value, false, CELL_PAD) {
                value_response.on_hover_text(value);
            }
        });
    };
    paint_pair(ui, "Name", "Value");
    egui::ScrollArea::vertical()
        .id_salt("dicom_metadata")
        .max_height(max_height)
        .show_rows_themed(ui, ROW_H, visible.len(), |ui, range| {
            for row in range {
                let (name, value) = &fields[visible[row]];
                paint_pair(ui, name, value);
            }
        });
}

fn output_text(out: &Outputs, name: &str) -> Result<String, String> {
    match out.get(name) {
        Some(Array::Character(text)) => text
            .to_utf8(&AtomicBool::new(false))
            .map_err(|error| error.to_string()),
        Some(Array::Str(text)) if text.len() == 1 => {
            Ok(text.data()[0].as_deref().unwrap_or("").to_owned())
        }
        _ => Err(format!("DICOM scan returned no {name}")),
    }
}
pub(crate) fn skipped_label(count: &Result<usize, String>) -> Option<String> {
    match count {
        Ok(0) => None,
        Ok(n) => Some(format!(
            "{n} file(s) in this folder could not be read as DICOM and were skipped"
        )),
        Err(reason) => Some(format!("Skipped-file count is unknown: {reason}")),
    }
}
pub(super) fn series_rows(
    table: &TableArray,
    dates: Option<&Array>,
    cancel: &AtomicBool,
) -> Result<Vec<Series>, String> {
    if table.height() > MAX_SERIES || table.width() != 14 {
        return Err("DICOM collection exceeds browser limits".into());
    }
    let text = |column: usize, row: usize| -> String {
        match table.column(column) {
            Array::Str(s) => s.data()[row].as_deref().unwrap_or("").to_owned(),
            _ => String::new(),
        }
    };
    let number = |column: usize, row: usize| -> usize {
        match table.column(column) {
            Array::Numeric(n) => n.data()[row] as usize,
            _ => 0,
        }
    };
    let mut rows = Vec::with_capacity(table.height());
    for row in 0..table.height() {
        if cancel.load(Ordering::Relaxed) {
            return Err("DICOM scan cancelled".into());
        }
        let date = match dates {
            Some(Array::Str(s)) => s.data()[row].as_deref().unwrap_or("").to_owned(),
            _ => String::new(),
        };
        let date = if date.trim().is_empty() || date.trim() == "NaT" {
            EMPTY_DATE.to_owned()
        } else {
            date
        };
        let (r, c, frames) = (number(9, row), number(10, row), number(12, row));
        rows.push(Series {
            uid: text(8, row),
            cells: [
                date,
                text(2, row),
                text(4, row),
                text(6, row),
                format!("{r} × {c}"),
                frames.to_string(),
            ],
            rows: r,
            cols: c,
            frames,
        });
    }
    Ok(rows)
}

fn read_collection(
    mut spec: Spec,
    out: &Outputs,
    skipped: Result<usize, String>,
    cancel: &AtomicBool,
) -> Result<Collection, String> {
    let Some(Array::Table(table)) = out.get("db_collection") else {
        return Err("DICOM scan returned no table".into());
    };
    let rows = series_rows(table, out.get("db_dates"), cancel)?;
    if spec.sample {
        let folder = output_text(out, "db_folder")?;
        if folder.is_empty() {
            return Err("DICOM sample did not report its folder".into());
        }
        spec.folder = folder;
    }
    Ok(Collection {
        spec,
        table: table.clone(),
        rows,
        skipped,
    })
}
fn info_number(info: &StructArray, element: usize, name: &str) -> Option<f64> {
    let value = info.get(info.field_index(name)?, element).ok()?;
    match value.as_ref() {
        Array::Numeric(n) => n.data().first().copied(),
        Array::Typed(n) if !n.is_empty() => Some(n.real(0)),
        _ => None,
    }
}
fn read_preview(uid: String, out: &Outputs, cancel: &AtomicBool) -> Result<Preview, String> {
    let volume = out.get("db_volume").ok_or("No DICOM volume")?.clone();
    let Some(Array::Struct(info)) = out.get("db_info") else {
        return Err("No DICOM metadata".into());
    };
    let dims = volume.dims();
    let frames = dims.get(3).copied().unwrap_or(1);
    if frames == 0 || !matches!(dims.get(2).copied().unwrap_or(1), 1 | 3) || dims.len() > 4 {
        return Err("Unsupported DICOM volume dimensions".into());
    }
    let mut metadata = vec![];
    let mut frame_info = vec![];
    if info.len().saturating_mul(info.field_count()) > MAX_METADATA_FIELDS {
        return Err("Selected series exceeds the browser metadata field limit".into());
    }
    let mut metadata_bytes = 0usize;
    for i in 0..info.len() {
        if cancel.load(Ordering::Relaxed) {
            return Err("DICOM read cancelled".into());
        }
        let count = info_number(info, i, "NumberOfFrames").unwrap_or(1.) as usize;
        if count > frames.saturating_sub(frame_info.len()) {
            return Err("DICOM frame metadata does not match volume".into());
        }
        frame_info.extend(std::iter::repeat_n(i, count));
        let mut fields = vec![];
        for (field, name) in info.field_names().iter().enumerate() {
            if field.is_multiple_of(64) && cancel.load(Ordering::Relaxed) {
                return Err("DICOM read cancelled".into());
            }
            let value = metadata_value(info.get(field, i)?, 0);
            metadata_bytes = metadata_bytes.saturating_add(name.len() + value.len());
            if metadata_bytes > MAX_METADATA_BYTES {
                return Err("Selected series exceeds the browser metadata display limit".into());
            }
            fields.push((name.clone(), value));
        }
        metadata.push(fields);
    }
    if frame_info.len() != frames {
        return Err("DICOM frame metadata does not match volume".into());
    }
    let mono16 = dims.get(2).copied().unwrap_or(1) == 1
        && info_number(info, 0, "BitsAllocated") == Some(16.);
    Ok(Preview {
        uid,
        volume,
        info: info.clone(),
        metadata,
        frame_info,
        frames,
        mono16,
    })
}

fn metadata_value(value: &Array, depth: usize) -> String {
    let text = if let Array::Struct(s) = value
        && s.len() == 1
        && depth < 2
    {
        s.field_names()
            .iter()
            .take(16)
            .enumerate()
            .filter_map(|(i, name)| {
                s.get(i, 0)
                    .ok()
                    .map(|v| format!("{name}: {}", metadata_value(v, depth + 1)))
            })
            .collect::<Vec<_>>()
            .join("; ")
    } else {
        value.preview(32)
    };
    text.chars().take(1024).collect()
}
/// Only a bounded, sampled display copy is made. Export retains the original
/// immutable typed volume; window/level changes only axes CLim.
fn frame_axes(
    volume: &Array,
    frame: usize,
    cancel: &AtomicBool,
) -> Result<(Axes, [f64; 2]), String> {
    let dims = volume.dims();
    let (rows, cols, channels) = (dims[0], dims[1], dims.get(2).copied().unwrap_or(1));
    if rows == 0 || cols == 0 || !matches!(channels, 1 | 3) {
        return Err("Unsupported DICOM image dimensions".into());
    }
    let step = rows.max(cols).div_ceil(DISPLAY_SIDE).max(1);
    let (h, w) = (rows.div_ceil(step), cols.div_ceil(step));
    let mut pixels = Vec::with_capacity(h * w * channels);
    let mut range = [f64::INFINITY, f64::NEG_INFINITY];
    let value_at = |at| -> Result<f64, String> {
        match volume {
            Array::Typed(n) => Ok(n.real(at)),
            Array::Numeric(n) => Ok(n.data()[at]),
            _ => Err("DICOM pixels must be numeric".into()),
        }
    };
    if channels == 1 {
        for at in 0..rows * cols {
            if at.is_multiple_of(4096) && cancel.load(Ordering::Relaxed) {
                return Err("DICOM display cancelled".into());
            }
            let value = value_at(at + rows * cols * frame)?;
            range = [range[0].min(value), range[1].max(value)];
        }
    }
    for channel in 0..channels {
        for col in (0..cols).step_by(step) {
            if cancel.load(Ordering::Relaxed) {
                return Err("DICOM display cancelled".into());
            }
            for row in (0..rows).step_by(step) {
                let at = row + rows * (col + cols * (channel + channels * frame));
                let value = value_at(at)?;
                range = [range[0].min(value), range[1].max(value)];
                pixels.push(value);
            }
        }
    }
    let scale = match volume {
        Array::Typed(n) if n.class() == matrust::typed_array::NumericClass::Uint16 => 65535.,
        _ => 255.,
    };
    let mapping = if channels == 3 {
        ImageMapping::TrueColor { full_scale: scale }
    } else {
        ImageMapping::Scaled
    };
    let image = ImageObject::new(h, w, pixels.into(), mapping);
    image.validate()?;
    let mut axes = Axes::default();
    axes.extras.images.push(image);
    axes.extras.y_reversed = true;
    axes.extras.colormap = Some(Arc::new((0..256).map(|i| [i as f64 / 255.; 3]).collect()));
    Ok((axes, range))
}

pub(crate) fn stop(host: &mut Host<DicomBrowser>) {
    host.runner.stop();
    host.delivered = None;
    let message =
        String::from("DICOM operation cancelled. Press Scan or select a series to continue.");
    host.app.job = None;
    host.app.inline_error = Some(message.clone());
    host.app.input_error = Some(message);
    host.app.running = false;
}

#[cfg(test)]
pub(crate) fn hold_scan_for_cancel_test(
    host: &mut Host<DicomBrowser>,
    ctx: &egui::Context,
    request: Request,
) -> std::sync::mpsc::Sender<()> {
    let (release, gate) = std::sync::mpsc::channel();
    host.runner.submit_task(
        ctx,
        request.key,
        Box::new(move |engine| {
            gate.recv_timeout(std::time::Duration::from_secs(10))
                .map_err(|e| e.to_string())?;
            super::run(engine, request)
        }),
    );
    release
}
pub(crate) fn show(
    host: &mut Host<DicomBrowser>,
    ui: &mut egui::Ui,
    ws: &Snapshot,
    busy: bool,
    action: &mut Option<Action>,
) {
    host.set_folder(ws.folder());
    host.app.running = host.running();
    host.show(ui, ws, busy, action);
    if let Err(error) = checked_names(&host.app.names) {
        host.script_error = Some(error);
    }
    if host.app.job.is_some() {
        host.app.inline_error = host.run_error.clone();
    }
    if std::mem::take(&mut host.app.stop_requested) {
        stop(host);
    }
    if !ui.ctx().will_discard() && std::mem::take(&mut host.app.pick_requested) {
        #[cfg(not(test))]
        {
            let mut dialog = rfd::FileDialog::new();
            if let Some(folder) = ws.folder() {
                dialog = dialog.set_directory(folder);
            }
            if let Some(folder) = dialog.pick_folder() {
                host.app.draft.folder = folder.to_string_lossy().into_owned();
            }
        }
    }
}
pub(crate) fn receive_open(workspace: &mut crate::Workspace) {
    let request = workspace
        .worker
        .dicom_browser
        .lock()
        .ok()
        .and_then(|mut slot| slot.take());
    if let Some(folder) = request
        && !workspace.worker.cancel.load(Ordering::Relaxed)
    {
        if let Some(folder) = folder {
            workspace.toolkits.apps.dicom.app.choose(folder);
        }
        workspace.open_app(crate::toolkits::Tool::App(AppKind::DicomBrowser));
    }
}
