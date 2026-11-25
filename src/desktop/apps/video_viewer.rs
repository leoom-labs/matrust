//! Clean-room video inspection. Public behavior reference:
//! https://www.mathworks.com/help/images/ref/videoviewer-app.html
//! One script builder serves private reads, workspace export and saved scripts.
use super::{App, AppKind, Host, Outputs, Snapshot, field};
use crate::toolkits::Action;
use eframe::egui;
use matrust::{
    Array,
    engine::implay::{Launch, shape},
    plot::Axes,
};
use std::{
    collections::{BTreeMap, VecDeque},
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

pub(crate) type OpenSlot = Arc<Mutex<Option<Launch>>>;
pub const CACHE_CAP: usize = 64 * 1024 * 1024;
const PREFETCH: usize = 4;
const FRAME_OVERHEAD: usize = 16 * 1024;
fn frame_bytes(elements: usize, class: &str) -> Result<usize, String> {
    let native = match class {
        "uint8" | "logical" => 1,
        "uint16" => 2,
        "single" => 4,
        _ => 8,
    };
    elements
        .checked_mul(native + 8)
        .and_then(|n| n.checked_add(FRAME_OVERHEAD))
        .ok_or_else(|| "Frame size overflow".into())
}

impl crate::Workspace {
    pub(crate) fn receive_video_viewer(&mut self) {
        let launch = self
            .worker
            .video_viewer
            .lock()
            .ok()
            .and_then(|mut s| s.take());
        if let Some(launch) = launch
            && !self
                .worker
                .cancel
                .load(std::sync::atomic::Ordering::Relaxed)
        {
            self.toolkits.apps.video.app.choose(launch);
            self.open_app(crate::toolkits::Tool::App(AppKind::VideoViewer));
        }
    }
}

#[derive(Clone)]
pub enum Source {
    File(String),
    Variable { name: String, array: Array },
}
impl Source {
    fn inputs(&self) -> Vec<(String, Array)> {
        match self {
            Self::File(_) => Vec::new(),
            Self::Variable { name, array } => vec![(name.clone(), array.clone())],
        }
    }
}

/// Statements that overwrite the synthetic sample and leave its path in `p`.
/// 90 frames of 160-by-120 RGB: a coloured ball crosses a dark field, and a
/// bar along the bottom grows with the frame index.
const SAMPLE_WRITE: &str = "\
p = fullfile(tempdir,'matrust-video-sample.avi');
warning('off','MATLAB:DELETE:FileNotFound');
delete(p);
warning('on','MATLAB:DELETE:FileNotFound');
w = VideoWriter(p,'Motion JPEG AVI');
w.FrameRate = 30;
open(w);
[xx,yy] = meshgrid(1:160,1:120);
for k = 1:90
t = (k-1)/89;
cx = 24+112*t;
cy = 40+24*sin(2*pi*t);
ball = (xx-cx).^2+(yy-cy).^2 <= 144;
bar = yy >= 112 & xx <= 8+144*t;
shade = double(ball);
grow = double(bar);
frame = cat(3,uint8(16+204*shade+24*grow),uint8(18+70*shade+182*grow),uint8(24+16*shade+40*grow));
writeVideo(w,frame);
end
close(w);
";

fn sample_program() -> String {
    format!(
        "vv_sample_path = matrust_video_sample();\nfunction p = matrust_video_sample()\n{SAMPLE_WRITE}end\n"
    )
}

fn sample_path(engine: &matrust::Engine) -> Result<String, String> {
    let Some(matrust::Array::Character(text)) = engine.array("vv_sample_path") else {
        return Err("Sample script did not report a path".into());
    };
    let path = String::from_utf16_lossy(text.units());
    if path.is_empty() {
        return Err("Sample script did not report a path".into());
    }
    super::comparison_tool::path_literal(&path)?;
    Ok(path)
}

/// The complete reproducing script. Reader state is local so Export never
/// overwrites an unrelated workspace reader or temporary variable.
/// A sample open repeats the writer, then opens that file.
pub fn frame_script(
    source: &Source,
    k: usize,
    output: &str,
    sample: bool,
) -> Result<String, String> {
    identifier(output)?;
    if k == 0 {
        return Err("Frame indices start at 1".into());
    }
    match source {
        Source::File(_) if sample => Ok(format!(
            "{output} = matrust_video_frame();\nfunction f = matrust_video_frame()\n{SAMPLE_WRITE}v = VideoReader(p);\nf = read(v, {k});\nend\n"
        )),
        Source::File(path) => {
            let path = super::comparison_tool::path_literal(path)?;
            Ok(format!(
                "{output} = matrust_video_frame();\nfunction f = matrust_video_frame()\nv = VideoReader({path});\nf = read(v, {k});\nend\n"
            ))
        }
        Source::Variable { name, array } => {
            identifier(name)?;
            let [_, _, _, frames] = shape(array)?;
            if k > frames {
                return Err("Frame index exceeds the sequence length".into());
            }
            let index = match array.dims().len() {
                2 => ":,:".into(),
                3 => format!(":,:,{k}"),
                _ => format!(":,:,:,{k}"),
            };
            Ok(format!("{output} = {name}({index});\n"))
        }
    }
}
fn identifier(name: &str) -> Result<(), String> {
    if name.len() > 63 || !matrust::files::valid_name(name) {
        return Err("Use a variable name of at most 63 characters".into());
    }
    super::check_export_name(name)?;
    matrust::syntax::parse(&format!("{name} = 1;")).map_err(|e| e.to_string())?;
    Ok(())
}

#[derive(Clone)]
pub struct Properties {
    pub height: usize,
    pub width: usize,
    pub frames: usize,
    pub fps: f64,
    pub format: String,
}
pub struct Frame {
    pub array: Array,
    pub axes: Axes,
    bytes: usize,
}
#[derive(Default)]
pub struct Cache {
    entries: BTreeMap<usize, Arc<Frame>>,
    order: VecDeque<usize>,
    pub bytes: usize,
}
impl Cache {
    fn insert(&mut self, k: usize, frame: Arc<Frame>) {
        if frame.bytes > CACHE_CAP {
            return;
        }
        if let Some(old) = self.entries.remove(&k) {
            self.bytes -= old.bytes;
            self.order.retain(|i| *i != k);
        }
        while self.bytes + frame.bytes > CACHE_CAP || self.entries.len() >= 128 {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            if let Some(old) = self.entries.remove(&oldest) {
                self.bytes -= old.bytes;
            }
        }
        self.bytes += frame.bytes;
        self.entries.insert(k, frame);
        self.order.push_back(k);
    }
    pub fn get(&self, k: usize) -> Option<Arc<Frame>> {
        self.entries.get(&k).cloned()
    }
}
pub struct Video {
    pub source: Source,
    pub properties: Properties,
    pub cache: Cache,
    /// True when this file was produced by Use sample data, so Generate script
    /// repeats the writer before opening it.
    pub sample: bool,
}
struct Delivery {
    opened: Option<(Source, Properties, bool)>,
    frames: Vec<(usize, Arc<Frame>)>,
}

pub struct VideoViewer {
    pub file_source: bool,
    pub path: String,
    pub variable: String,
    pub fps: f64,
    pub output_name: String,
    pub replace: bool,
    pub current: Option<Video>,
    pub shown: Option<Arc<Frame>>,
    pub index: usize,
    pub shown_index: usize,
    pub playing: bool,
    pub looping: bool,
    pub speed: f64,
    pub fit: bool,
    pub achieved_fps: f64,
    pub error: Option<String>,
    pub open_requested: bool,
    sample_requested: bool,
    cancel_requested: bool,
    browse_requested: bool,
    opening: bool,
    running: bool,
    serial: u64,
    delivered: Option<u64>,
    payload: Arc<Mutex<Option<Delivery>>>,
    launch_source: Option<Source>,
    anchor: Option<(f64, usize)>,
    measured: Option<(f64, usize)>,
    #[cfg(test)]
    pub task_gate: Option<Arc<std::sync::atomic::AtomicBool>>,
    #[cfg(test)]
    pub controls: BTreeMap<&'static str, egui::Rect>,
}
impl Default for VideoViewer {
    fn default() -> Self {
        Self {
            file_source: true,
            path: String::new(),
            variable: String::new(),
            fps: 20.,
            output_name: "video_frame".into(),
            replace: false,
            current: None,
            shown: None,
            index: 1,
            shown_index: 0,
            playing: false,
            looping: false,
            speed: 1.,
            fit: true,
            achieved_fps: 0.,
            error: None,
            open_requested: false,
            sample_requested: false,
            cancel_requested: false,
            browse_requested: false,
            opening: false,
            running: false,
            serial: 0,
            delivered: None,
            payload: Default::default(),
            launch_source: None,
            anchor: None,
            measured: None,
            #[cfg(test)]
            controls: BTreeMap::new(),
            #[cfg(test)]
            task_gate: None,
        }
    }
}
impl VideoViewer {
    pub fn choose(&mut self, launch: Launch) {
        match launch {
            Launch::Empty => return,
            Launch::File(path) => {
                self.file_source = true;
                self.path = path.to_string_lossy().into_owned();
            }
            Launch::Array { name, array, fps } => {
                self.file_source = false;
                self.variable = name.clone();
                self.fps = fps;
                self.launch_source = Some(Source::Variable { name, array });
            }
        }
        self.open_requested = true;
    }
    fn draft_source(&mut self, ws: &Snapshot) -> Result<Source, String> {
        if !self.file_source && (!self.fps.is_finite() || self.fps <= 0.) {
            return Err("Frame rate must be positive and finite".into());
        }
        if self.file_source {
            let path = Path::new(&self.path);
            let path = if path.is_relative() {
                ws.folder()
                    .map_or_else(|| path.to_owned(), |f| f.join(path))
            } else {
                path.to_owned()
            };
            let path = path.to_str().ok_or("Paths must be Unicode")?.to_owned();
            super::comparison_tool::path_literal(&path)?;
            self.launch_source = None;
            Ok(Source::File(path))
        } else {
            identifier(&self.variable)?;
            let array = ws
                .get(&self.variable)
                .cloned()
                .or_else(|| match self.launch_source.take() {
                    Some(Source::Variable { name, array }) if name == self.variable => Some(array),
                    _ => None,
                })
                .ok_or("Choose a workspace sequence")?;
            shape(&array)?;
            Ok(Source::Variable {
                name: self.variable.clone(),
                array,
            })
        }
    }
    fn output(&self, ws: &Snapshot) -> Result<String, String> {
        identifier(&self.output_name)?;
        if matches!(
            self.output_name.as_str(),
            "matrust_video_frame" | "VideoReader" | "read" | "implay"
        ) {
            return Err("That name is reserved by Video Viewer".into());
        }
        if ws.contains(&self.output_name) && !self.replace {
            return Err(format!(
                "{} exists; choose another name or enable Replace",
                self.output_name
            ));
        }
        if let Some(Video {
            source: Source::Variable { name, array },
            ..
        }) = &self.current
        {
            if *name == self.output_name {
                return Err("The export name must differ from the source variable".into());
            }
            if !ws.get(name).is_some_and(|a| a.same_snapshot(array)) {
                return Err("The source variable changed; Open it again before exporting".into());
            }
        }
        Ok(self.output_name.clone())
    }
    fn record(&mut self, name: &'static str, response: &egui::Response) {
        #[cfg(test)]
        self.controls.insert(name, response.rect);
        #[cfg(not(test))]
        let _ = (name, response);
    }
    fn button(&mut self, ui: &mut egui::Ui, label: &'static str) -> bool {
        let r = ui.button(label);
        self.record(label, &r);
        r.clicked()
    }
    pub fn seek(&mut self, index: usize) {
        if let Some(video) = &self.current {
            self.index = index.clamp(1, video.properties.frames);
            self.anchor = None;
        }
    }
    fn toggle_play(&mut self) {
        self.playing = !self.playing;
        if !self.playing {
            self.index = self.shown_index.max(1);
        }
        if self.playing
            && self
                .current
                .as_ref()
                .is_some_and(|v| self.index == v.properties.frames)
        {
            self.index = 1;
        }
        self.anchor = None;
        self.measured = None;
        self.achieved_fps = 0.;
    }
    pub fn tick(&mut self, now: f64, ctx: &egui::Context) {
        if !self.playing {
            self.anchor = None;
            return;
        }
        let Some(video) = &self.current else {
            self.playing = false;
            return;
        };
        let rate = (video.properties.fps * self.speed).min(f64::MAX);
        let (start, first) = *self.anchor.get_or_insert((now, self.index));
        let steps = (((now - start).max(0.) * rate) + 1e-9).floor() as usize;
        let wanted = first.saturating_sub(1).saturating_add(steps);
        self.index = if self.looping {
            wanted % video.properties.frames + 1
        } else {
            wanted.saturating_add(1).min(video.properties.frames)
        };
        if !self.looping && wanted >= video.properties.frames {
            self.playing = false;
        }
        let (since, count) = *self.measured.get_or_insert((now, 0));
        if now > since {
            self.achieved_fps = count as f64 / (now - since);
        }
        // Wake on the next frame boundary, with no accumulating decode queue.
        let delay = ((steps as f64 + 1.) / rate - (now - start)).clamp(0.001, 1.);
        ctx.request_repaint_after(Duration::from_secs_f64(delay));
    }
    fn display_cached(&mut self) {
        let Some(video) = &self.current else {
            return;
        };
        let frame = video
            .cache
            .get(self.index)
            .map(|f| (self.index, f))
            .or_else(|| {
                // A slow decode still presents its newest usable frame while the
                // clock advances. The next read jumps directly to the latest target.
                if self.playing {
                    video
                        .cache
                        .entries
                        .range(..=self.index)
                        .next_back()
                        .filter(|(k, _)| **k > self.shown_index)
                        .map(|(k, frame)| (*k, frame.clone()))
                } else {
                    None
                }
            });
        if let Some((k, frame)) = frame
            && self.shown_index != k
        {
            self.shown = Some(frame);
            self.shown_index = k;
            if let Some((_, count)) = &mut self.measured {
                *count += 1;
            }
        }
    }
}

fn properties(
    engine: &mut matrust::Engine,
    source: &Source,
    fps: f64,
) -> Result<Properties, String> {
    match source {
        Source::Variable { array, .. } => {
            let [height, width, channels, frames] = shape(array)?;
            Ok(Properties {
                height,
                width,
                frames,
                fps,
                format: format!(
                    "{} {}",
                    if channels == 3 { "RGB" } else { "Gray" },
                    array.class()
                ),
            })
        }
        Source::File(path) => {
            let path = super::comparison_tool::path_literal(path)?;
            engine.execute(&format!("v = VideoReader({path}); vv_meta = [v.Height v.Width v.NumFrames v.FrameRate]; vv_format = v.VideoFormat;"))?;
            let Some(Array::Numeric(meta)) = engine.array("vv_meta") else {
                return Err("Missing video properties".into());
            };
            let m = meta.data();
            Ok(Properties {
                height: m[0] as usize,
                width: m[1] as usize,
                frames: m[2] as usize,
                fps: m[3],
                format: engine
                    .array("vv_format")
                    .map_or_else(|| "AVI".into(), |a| a.to_string()),
            })
        }
    }
}
fn decode(engine: &mut matrust::Engine, source: &Source, k: usize) -> Result<Arc<Frame>, String> {
    // Isolate all source names from the private display output name.
    let output = match source {
        Source::Variable { name, .. } if name == "vv_frame" => "vv_frame_result",
        _ => "vv_frame",
    };
    let script = frame_script(source, k, output, false)?;
    let (body, functions) = super::split_local_functions(&script);
    engine.execute(&format!("{body}imshow({output});\n{functions}"))?;
    let array = engine
        .array(output)
        .ok_or("Frame script did not produce an array")?;
    let axes = engine
        .figures()
        .next()
        .ok_or("Frame did not produce an image")?
        .1
        .axes
        .clone();
    // Conservative accounting: native array plus the f64 display planes,
    // axes metadata and cache entry. Shared arrays never copy on assignment.
    let bytes = frame_bytes(array.len(), array.class())?;
    if bytes > CACHE_CAP {
        return Err("Frame exceeds the 64 MiB display cache limit".into());
    }
    Ok(Arc::new(Frame { array, axes, bytes }))
}
fn submit(
    host: &mut Host<VideoViewer>,
    ctx: &egui::Context,
    source: Option<Source>,
    open: bool,
    k: usize,
) {
    host.app.serial += 1;
    host.app.opening = open;
    let serial = host.app.serial;
    let payload = host.app.payload.clone();
    let fps = host.app.fps;
    let known = host.app.current.as_ref().map(|v| v.properties.clone());
    #[cfg(test)]
    let gate = host.app.task_gate.clone();
    host.runner.submit_task(
        ctx,
        serial,
        Box::new(move |engine| {
            #[cfg(test)]
            if let Some(gate) = gate {
                while !gate.load(std::sync::atomic::Ordering::Relaxed) {
                    matrust::dsp::check_cancel(&engine.cancellation())?;
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
            // None writes the sample in this same job, then opens that file.
            let (source, sample) = match source {
                None => {
                    engine.execute(&sample_program())?;
                    (Source::File(sample_path(engine)?), true)
                }
                Some(source) => (source, false),
            };
            // A private input name prevents workspace arrays named after
            // display builtins from shadowing the off-thread image probe.
            let decode_source = match &source {
                Source::Variable { array, .. } => Source::Variable {
                    name: "vv_source".into(),
                    array: array.clone(),
                },
                Source::File(path) => Source::File(path.clone()),
            };
            for (name, array) in decode_source.inputs() {
                engine.insert_array(&name, array)?;
            }
            let props = if open {
                properties(engine, &source, fps)?
            } else {
                known.ok_or("Open a video first")?
            };
            if props.frames == 0 || !props.fps.is_finite() || props.fps <= 0. {
                return Err("Video has no frames or valid frame rate".into());
            }
            let channels = match &source {
                Source::Variable { array, .. } => shape(array)?[2],
                Source::File(_) => 3,
            };
            let elements = props
                .height
                .checked_mul(props.width)
                .and_then(|n| n.checked_mul(channels))
                .ok_or("Frame size overflow")?;
            let class = match &source {
                Source::Variable { array, .. } => array.class(),
                Source::File(_) => "uint8",
            };
            if frame_bytes(elements, class)? > CACHE_CAP {
                return Err("Frame exceeds the 64 MiB display cache limit".into());
            }
            let first = decode(engine, &decode_source, k)?;
            let count = PREFETCH
                .min((CACHE_CAP / first.bytes).max(1))
                .min(props.frames - k + 1);
            let mut frames = vec![(k, first)];
            for index in k + 1..k + count {
                matrust::dsp::check_cancel(&engine.cancellation())?;
                // A later damaged frame must not discard a successfully read one.
                match decode(engine, &decode_source, index) {
                    Ok(frame) => frames.push((index, frame)),
                    Err(_) => break,
                }
            }
            matrust::dsp::check_cancel(&engine.cancellation())?;
            *payload.lock().map_err(|_| "Video delivery lock failed")? = Some(Delivery {
                opened: open.then_some((source, props, sample)),
                frames,
            });
            Ok(Outputs::new())
        }),
    );
}

pub(crate) fn refresh(host: &mut Host<VideoViewer>, ctx: &egui::Context, ws: &Snapshot) {
    host.set_folder(ws.folder());
    host.runner.poll(ctx);
    if host.app.delivered != host.runner.key()
        && let Some(result) = host.runner.result()
    {
        host.app.delivered = host.runner.key();
        match result {
            Ok(_) => {
                let delivery = host.app.payload.lock().ok().and_then(|mut s| s.take());
                if let Some(delivery) = delivery {
                    if let Some((source, properties, sample)) = delivery.opened {
                        if sample && let Source::File(path) = &source {
                            host.app.file_source = true;
                            host.app.path = path.clone();
                            host.app.launch_source = None;
                        }
                        host.app.current = Some(Video {
                            source,
                            properties,
                            cache: Cache::default(),
                            sample,
                        });
                        host.app.index = 1;
                        host.app.shown_index = 0;
                        host.app.playing = false;
                        host.app.anchor = None;
                    }
                    if let Some(video) = &mut host.app.current {
                        for (k, frame) in delivery.frames {
                            video.cache.insert(k, frame);
                        }
                    }
                    host.app.error = None;
                    host.run_error = None;
                }
            }
            Err(error) => {
                host.app.error = Some(error.clone());
                host.run_error = Some(error.clone());
            }
        }
        host.app.opening = false;
    }
    let now = ctx.input(|i| i.time);
    host.app.tick(now, ctx);
    host.app.display_cached();
    let sample = std::mem::take(&mut host.app.sample_requested);
    let open = std::mem::take(&mut host.app.open_requested);
    if sample || open {
        host.runner.stop();
        host.app.delivered = None;
        *host.app.payload.lock().unwrap() = None;
        if sample {
            host.app.error = None;
            host.run_error = None;
            submit(host, ctx, None, true, 1);
        } else {
            match host.app.draft_source(ws) {
                Ok(source) => {
                    host.app.error = None;
                    host.run_error = None;
                    submit(host, ctx, Some(source), true, 1);
                }
                Err(error) => {
                    host.run_error = Some(error.clone());
                    host.app.error = Some(error);
                }
            }
        }
    } else if !host.runner.running()
        && host.app.current.is_some()
        && host.app.shown_index != host.app.index
    {
        // One in-flight batch, no queued seeks. Time continues while decoding;
        // completion displays only the newest target and skips stale frames.
        let source = host.app.current.as_ref().unwrap().source.clone();
        // A failed target is retried by another transport action, not every UI frame.
        if host.app.error.is_none() {
            submit(host, ctx, Some(source), false, host.app.index);
        }
    }
    host.app.running = host.runner.running();
}
pub(crate) fn stop(host: &mut Host<VideoViewer>) {
    host.runner.stop();
    host.app.delivered = None;
    host.app.opening = false;
    host.app.playing = false;
    host.app.index = host.app.shown_index.max(1);
    host.app.open_requested = false;
    host.app.sample_requested = false;
    host.app.anchor = None;
    *host.app.payload.lock().unwrap() = None;
    host.message = "Cancelled; previous video retained".into();
}

impl App for VideoViewer {
    fn kind(&self) -> AppKind {
        AppKind::VideoViewer
    }
    fn script(&self, ws: &Snapshot) -> Result<String, String> {
        let name = self.output(ws)?;
        let video = self.current.as_ref().ok_or("Open a video first")?;
        if self.shown_index == 0 {
            return Err("Read a frame first".into());
        }
        frame_script(&video.source, self.shown_index, &name, video.sample)
    }
    fn inputs(&self) -> Vec<String> {
        self.current.as_ref().map_or_else(Vec::new, |v| {
            v.source.inputs().into_iter().map(|(n, _)| n).collect()
        })
    }
    fn probe(&self, _: &Snapshot) -> (String, Vec<String>) {
        (String::new(), Vec::new())
    }
    fn receive(&mut self, _: &Outputs) -> Result<(), String> {
        Ok(())
    }
    fn inputs_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.file_source, true, "File");
            ui.selectable_value(&mut self.file_source, false, "Workspace");
        });
        ui.add_space(8.);
        if self.file_source {
            field(ui, "Video file", |ui| {
                ui.add(
                    crate::tokens::input(
                        egui::TextEdit::singleline(&mut self.path).char_limit(4096),
                    )
                    .desired_width(ui.available_width()),
                );
            });
            if self.button(ui, "Browse…") {
                self.browse_requested = true;
            }
        } else {
            field(ui, "Sequence", |ui| {
                egui::ComboBox::from_id_salt("video_variable")
                    .width(ui.available_width())
                    .selected_text(if self.variable.is_empty() {
                        "Choose a variable"
                    } else {
                        &self.variable
                    })
                    .show_ui(ui, |ui| {
                        for (name, array) in ws.variables().filter(|(_, a)| shape(a).is_ok()) {
                            let _ = array;
                            ui.selectable_value(&mut self.variable, name.into(), name);
                        }
                    });
            });
            field(ui, "Frame rate (fps)", |ui| {
                ui.add(egui::DragValue::new(&mut self.fps).speed(1.));
            });
        }
        ui.add_space(8.);
        ui.horizontal_wrapped(|ui| {
            if self.button(ui, "Open") {
                self.open_requested = true;
            }
            if self.button(ui, "Use sample data") {
                self.sample_requested = true;
            }
            let r = ui.add_enabled(self.running, egui::Button::new("Cancel"));
            self.record("Cancel", &r);
            if r.clicked() {
                self.cancel_requested = true;
            }
        });
        if let Some(v) = &self.current {
            let p = &v.properties;
            ui.add_space(16.);
            ui.label("Properties");
            super::result_block(
                ui,
                &[
                    ("Size", format!("{} × {}", p.width, p.height)),
                    ("Frames", p.frames.to_string()),
                    ("Frame rate", format!("{} fps", super::short(p.fps))),
                    (
                        "Duration",
                        format!("{} s", super::short(p.frames as f64 / p.fps)),
                    ),
                    ("Format", p.format.clone()),
                ],
            );
        }
    }
    fn main_caption(&self) -> &'static str {
        "VIDEO"
    }
    fn main_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.fit, true, "Fit");
            ui.selectable_value(&mut self.fit, false, "100%");
        });
        ui.add_space(8.);
        if let Some(error) = &self.error {
            ui.colored_label(crate::tokens::palette(ui.ctx()).error, error);
        }
        let width = ui.available_width().max(64.);
        let height = (ui.available_height() - 128.).max(240.);
        if let Some(frame) = &self.shown {
            let h = frame.array.dims()[0] as f32;
            let w = frame.array.dims()[1] as f32;
            let scale = if self.fit {
                (width / w).min(height / h).max(0.001)
            } else {
                1.
            };
            let size = egui::vec2(w * scale, h * scale);
            egui::ScrollArea::both()
                .id_salt("video_image_scroll")
                .max_height(height)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let id = egui::Id::new("video_frame_image");
                    egui_plot::Plot::new(id)
                        .width(size.x)
                        .height(size.y)
                        .show_axes([false, false])
                        .show_grid([false, false])
                        .show_background(false)
                        .allow_zoom(false)
                        .allow_scroll(false)
                        .allow_drag(false)
                        .allow_boxed_zoom(false)
                        .allow_double_click_reset(false)
                        .show(ui, |plot| {
                            plot.set_plot_bounds(egui_plot::PlotBounds::from_min_max(
                                [0.5, 0.5],
                                [w as f64 + 0.5, h as f64 + 0.5],
                            ));
                            crate::figure_images::add(plot, &frame.axes, id);
                        });
                });
        } else {
            ui.allocate_ui(egui::vec2(width, height), |ui| {
                ui.centered_and_justified(|ui| {
                    ui.label("Open a video file or workspace sequence");
                });
            });
        }
        ui.add_space(8.);
        let Some(video) = &self.current else {
            return;
        };
        let frames = video.properties.frames;
        let fps = video.properties.fps;
        ui.add_enabled_ui(!self.opening, |ui| {
            ui.horizontal_wrapped(|ui| {
                if self.button(ui, "First") {
                    self.seek(1);
                    self.error = None;
                }
                if self.button(ui, "Back") {
                    self.seek(self.index.saturating_sub(1));
                    self.error = None;
                }
                if self.button(ui, if self.playing { "Pause" } else { "Play" }) {
                    self.toggle_play();
                    self.error = None;
                }
                if self.button(ui, "Step") {
                    self.seek(self.index + 1);
                    self.error = None;
                }
                if self.button(ui, "Last") {
                    self.seek(frames);
                    self.error = None;
                }
                ui.checkbox(&mut self.looping, "Loop");
                let old = self.speed;
                egui::ComboBox::from_id_salt("video_speed")
                    .selected_text(format!("{}x", self.speed))
                    .show_ui(ui, |ui| {
                        for speed in [0.25, 0.5, 1., 2., 4.] {
                            ui.selectable_value(&mut self.speed, speed, format!("{speed}x"));
                        }
                    });
                if old != self.speed {
                    self.anchor = None;
                    self.measured = None;
                }
            });
            // A focusable track with explicit key handling and a visible focus ring.
            let (rect, r) = ui.allocate_exact_size(
                egui::vec2(ui.available_width(), 24.),
                egui::Sense::click_and_drag(),
            );
            r.widget_info(|| {
                egui::WidgetInfo::slider(ui.is_enabled(), self.index as f64, "Frame track")
            });
            self.record("Track", &r);
            let rail = rect.shrink2(egui::vec2(8., 8.));
            let palette = crate::tokens::palette(ui.ctx());
            ui.painter().rect_filled(rail, 4., palette.surface);
            let fraction = (self.index - 1) as f32 / frames.saturating_sub(1).max(1) as f32;
            let x = rail.left() + fraction * rail.width();
            ui.painter()
                .circle_filled(egui::pos2(x, rect.center().y), 6., palette.accent);
            if r.clicked() || r.dragged() {
                r.request_focus();
                if let Some(pos) = r.interact_pointer_pos() {
                    let fraction = ((pos.x - rail.left()) / rail.width()).clamp(0., 1.);
                    self.seek(1 + (fraction * frames.saturating_sub(1) as f32).round() as usize);
                    self.error = None;
                }
            }
            if r.has_focus() {
                ui.memory_mut(|memory| {
                    memory.set_focus_lock_filter(
                        r.id,
                        egui::EventFilter {
                            horizontal_arrows: true,
                            ..Default::default()
                        },
                    )
                });
                ui.painter().rect_stroke(
                    r.rect.expand(4.),
                    4.,
                    egui::Stroke::new(1., crate::tokens::palette(ui.ctx()).accent),
                    egui::StrokeKind::Outside,
                );
                let key = ui.input(|i| {
                    [
                        egui::Key::ArrowLeft,
                        egui::Key::ArrowRight,
                        egui::Key::Home,
                        egui::Key::End,
                        egui::Key::Space,
                    ]
                    .into_iter()
                    .find(|k| i.key_pressed(*k))
                });
                if let Some(key) = key {
                    match key {
                        egui::Key::ArrowLeft => self.seek(self.index.saturating_sub(1)),
                        egui::Key::ArrowRight => self.seek(self.index + 1),
                        egui::Key::Home => self.seek(1),
                        egui::Key::End => self.seek(frames),
                        _ => self.toggle_play(),
                    }
                    self.error = None;
                }
            }
            ui.label(format!(
                "{} / {}    {:.3} s    {:.1} fps achieved",
                self.index,
                frames,
                (self.index - 1) as f64 / fps,
                self.achieved_fps
            ));
            if self.shown_index != self.index {
                crate::tokens::helper(
                    ui,
                    format!(
                        "Showing frame {} while reading {}",
                        self.shown_index, self.index
                    ),
                );
            }
        });
    }
    fn results(&self) -> Vec<(&'static str, String)> {
        vec![("Current frame", self.shown_index.to_string())]
    }
    fn names_ui(&mut self, ui: &mut egui::Ui, _: &Snapshot) {
        field(ui, "Frame variable", |ui| {
            ui.add(
                crate::tokens::input(
                    egui::TextEdit::singleline(&mut self.output_name).char_limit(63),
                )
                .desired_width(ui.available_width()),
            );
        });
        ui.checkbox(&mut self.replace, "Replace existing variable");
    }
    fn exported(&self, _: &Snapshot) -> Vec<String> {
        vec![self.output_name.clone()]
    }
    fn summary(&self) -> Option<String> {
        self.current.as_ref().map(|v| {
            format!(
                "Video Viewer: frame {} of {}",
                self.shown_index, v.properties.frames
            )
        })
    }
    fn export_label(&self) -> &'static str {
        "Export current frame"
    }
}

pub(crate) fn show(
    host: &mut Host<VideoViewer>,
    ui: &mut egui::Ui,
    ws: &Snapshot,
    busy: bool,
    action: &mut Option<Action>,
) {
    refresh(host, ui.ctx(), ws);
    let mut export = false;
    let mut generate = false;
    super::three_areas(ui, AppKind::VideoViewer, "VIDEO", |ui, area| match area {
        super::Area::Inputs => host.app.inputs_ui(ui, ws),
        super::Area::Main => host.app.main_ui(ui),
        super::Area::Output => {
            super::result_block(ui, &host.app.results());
            ui.add_space(16.);
            host.app.names_ui(ui, ws);
            ui.add_space(8.);
            let result = host.app.script(ws);
            let ready = !busy
                && !host.saving
                && !host.app.opening
                && host.app.shown.is_some()
                && result.is_ok();
            let r = ui.add_enabled(ready, egui::Button::new("Export current frame"));
            host.app.record("Export", &r);
            export = r.clicked();
            let r = ui.add_enabled(ready, egui::Button::new("Generate script"));
            host.app.record("Generate script", &r);
            generate = r.clicked();
            if let Err(error) = result {
                crate::tokens::helper(ui, error);
            }
            if !host.message.is_empty() {
                crate::tokens::helper(ui, &host.message);
            }
        }
    });
    if export {
        *action = host.export(ws);
    }
    if generate {
        *action = host.generate(ws, super::now());
    }
    if std::mem::take(&mut host.app.cancel_requested) {
        stop(host);
    }
    if !ui.ctx().will_discard()
        && std::mem::take(&mut host.app.browse_requested)
        && let Some(path) = pick_path(ws)
    {
        host.app.path = path;
    }
    // Process clicks in this frame; tick only advances on a later timestamp.
    refresh(host, ui.ctx(), ws);
    host.script_error = None;
    host.run_error = host.app.error.clone();
}
fn pick_path(ws: &Snapshot) -> Option<String> {
    #[cfg(test)]
    {
        let _ = ws;
        None
    }
    #[cfg(not(test))]
    {
        let mut dialog = rfd::FileDialog::new()
            .add_filter("AVI video", &["avi"])
            .add_filter("All files", &["*"]);
        if let Some(folder) = ws.folder() {
            dialog = dialog.set_directory(folder);
        }
        dialog.pick_file().map(|p| p.to_string_lossy().into_owned())
    }
}
