//! Fixed, opt-in native EPS acceptance cases. No generic command execution or OS input.
use super::*;
use std::fs::{self, OpenOptions};
use std::io::Write;

const RED: &[u8] = b"%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 10 20 50 40\n1 0 0 setrgbcolor 10 20 40 20 rectfill showpage\n%%EOF\n";
const BLUE: &[u8] = b"%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 10 20 50 40\n0 0 1 setrgbcolor 10 20 40 20 rectfill showpage\n%%EOF\n";
const INVALID: &[u8] = b"%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 10 10\nmatrust_intentional_undefined_operator showpage\n";
const LOOP: &[u8] = b"%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 10 10\n{} loop\n";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Case {
    Open,
    Workflow,
    Settings,
    Dormant,
    Reopen,
}
pub(super) struct Config {
    directory: PathBuf,
    case: Case,
}
impl Config {
    pub(super) fn parse(
        args: Vec<String>,
        options: &screenshot_options::Options,
    ) -> Result<Option<Self>, String> {
        let indices: Vec<_> = args
            .iter()
            .enumerate()
            .filter(|(_, a)| a.as_str() == "--eps-native-case")
            .map(|(i, _)| i)
            .collect();
        if indices.is_empty() {
            return Ok(None);
        }
        if indices.len() != 1 {
            return Err("Supply one EPS capture case".into());
        }
        let case = match args.get(indices[0] + 1).map(String::as_str) {
            Some("open") => Case::Open,
            Some("workflow") => Case::Workflow,
            Some("settings") => Case::Settings,
            Some("recovery-dormant") => Case::Dormant,
            Some("recovery-reopen") => Case::Reopen,
            _ => return Err("Unknown EPS capture case".into()),
        };
        if args.iter().any(|a| {
            matches!(
                a.as_str(),
                "--isolated"
                    | "--open-files"
                    | "--preview-image"
                    | "--preview-page"
                    | "--preview-pdf-page"
            ) || (a.starts_with("--") && a.ends_with("-demo") && a != "--compact-demo")
                || a.starts_with("--detached-")
        }) || options.script_reload_demo
            || options.model_new_demo
            || options.figure_selector_demo
            || options.iir_designer_case.is_some()
        {
            return Err(
                "EPS capture requires its private session without other demo/open actions".into(),
            );
        }
        let directory = std::env::current_dir()
            .and_then(fs::canonicalize)
            .map_err(|e| e.to_string())?;
        let session = std::env::var_os("MATRUST_SESSION_PATH")
            .map(PathBuf::from)
            .ok_or("EPS capture requires a private session path")?;
        let screenshot = options
            .path
            .as_ref()
            .ok_or("EPS capture requires --screenshot")?;
        for path in [&session, screenshot] {
            if !path.is_absolute()
                || path
                    .parent()
                    .and_then(|p| fs::canonicalize(p).ok())
                    .as_ref()
                    != Some(&directory)
            {
                return Err("EPS capture session and screenshot must be directly inside its explicit private working directory".into());
            }
        }
        if session == *screenshot
            || screenshot
                .extension()
                .and_then(|e| e.to_str())
                .is_none_or(|e| !e.eq_ignore_ascii_case("png"))
        {
            return Err("EPS capture requires distinct session and PNG destinations".into());
        }
        if session.exists()
            && fs::canonicalize(&session)
                .ok()
                .and_then(|p| p.parent().map(PathBuf::from))
                .as_ref()
                != Some(&directory)
        {
            return Err("Private session may not escape the working directory".into());
        }
        if screenshot.exists() {
            return Err("EPS screenshot destination must be fresh".into());
        }
        if matches!(case, Case::Dormant | Case::Reopen)
            && (!session.is_file() || std::env::var_os("MATRUST_EPS_RENDERER").is_some())
        {
            return Err(
                "Recovery capture requires a real saved private session and no renderer override"
                    .into(),
            );
        }
        Ok(Some(Self { directory, case }))
    }
    pub(super) fn start(self) -> Result<Coordinator, String> {
        let owned = self.directory.join(format!("eps-capture-{:?}", self.case));
        fs::create_dir(&owned).map_err(|e| format!("Fresh EPS capture directory required: {e}"))?;
        Ok(Coordinator {
            config: self,
            owned,
            started: Instant::now(),
            stage: 0,
            frame: None,
            baseline: None,
            figures: BTreeMap::new(),
            current: 1,
            history: BTreeMap::new(),
            documents: BTreeMap::new(),
            previous: None,
            events: Vec::new(),
            ready: false,
            stage_started: Instant::now(),
            dormant_frames: 0,
        })
    }
}
pub(super) struct Coordinator {
    config: Config,
    owned: PathBuf,
    started: Instant,
    stage_started: Instant,
    stage: u8,
    frame: Option<u64>,
    baseline: Option<Session>,
    figures: BTreeMap<u32, Figure>,
    current: u32,
    history: BTreeMap<u32, (usize, usize)>,
    documents: BTreeMap<u32, worker::FigureStatus>,
    previous: Option<Arc<egui::ColorImage>>,
    events: Vec<serde_json::Value>,
    ready: bool,
    dormant_frames: u8,
}
fn require(ok: bool, message: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(message.into()) }
}
fn same_script_documents(
    actual: &[matrust::document::Document],
    saved: &[matrust::document::Document],
) -> Result<bool, String> {
    // IDs are skipped by serialization; serialized state includes the private
    // saved-text baseline as well as visible contents, paths and dirty flags.
    Ok(actual.iter().map(|d| d.id).eq(saved.iter().map(|d| d.id))
        && serde_json::to_vec(actual).map_err(|e| e.to_string())?
            == serde_json::to_vec(saved).map_err(|e| e.to_string())?)
}
impl Coordinator {
    pub(super) fn ready(&self) -> bool {
        self.ready
    }
    fn transition(&mut self, stage: u8, label: &str, app: &Workspace) -> Result<(), String> {
        require(self.events.len() < 32, "EPS capture transition bound")?;
        self.check_invariants(app)?;
        self.events.push(
            serde_json::json!({"stage":stage,"action":label,"busy":app.busy,
            "preview":app.image_view.as_ref().map(|v| (&v.path,v.preview_size)),
            "pending":app.eps_preview.pending,"error":app.eps_preview.error}),
        );
        self.stage = stage;
        self.stage_started = Instant::now();
        Ok(())
    }
    fn check_invariants(&self, app: &Workspace) -> Result<(), String> {
        if let Some(saved) = &self.baseline {
            let actual = app.snapshot();
            require(
                app.current_figure == self.current
                    && app.figure_history == self.history
                    && app.figure_documents == self.documents,
                "EPS changed figure selection/history/documents",
            )?;
            require(
                same_script_documents(&actual.documents, &saved.documents)?
                    && actual.active == saved.active,
                "EPS changed script documents",
            )?;
            require(
                serde_json::to_vec(&actual.model).map_err(|e| e.to_string())?
                    == serde_json::to_vec(&saved.model).map_err(|e| e.to_string())?,
                "EPS changed block model",
            )?;
            let vars = saved
                .workspace
                .as_ref()
                .ok_or("Missing baseline workspace")?;
            require(
                vars.len() == app.variables.len()
                    && vars
                        .iter()
                        .all(|(n, v)| app.variables.get(n).is_some_and(|a| v.same_snapshot(a))),
                "EPS changed shared workspace",
            )?;
            require(
                self.figures.len() == app.all_figures.len()
                    && self
                        .figures
                        .iter()
                        .all(|(n, f)| app.all_figures.get(n).is_some_and(|a| f.same_snapshot(a))),
                "EPS changed figure snapshots",
            )?;
        }
        Ok(())
    }
    fn pixels(
        &self,
        app: &Workspace,
        expected: egui::Color32,
    ) -> Result<Arc<egui::ColorImage>, String> {
        let view = app
            .image_view
            .as_ref()
            .ok_or("Expected actual worker preview")?;
        require(
            view.path
                .extension()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.eq_ignore_ascii_case("eps"))
                && view.error.is_none(),
            "Expected successful EPS preview",
        )?;
        let p = &view.pixels;
        let [w, h] = p.size;
        require(
            w >= 16 && h >= 16 && p.pixels.len() == w * h,
            "Invalid EPS preview dimensions",
        )?;
        require(
            p.pixels[(h / 2) * w + w / 2] == expected
                && p.pixels[2 * w + w / 2] == egui::Color32::WHITE,
            "Actual interpreted EPS pixels differ",
        )?;
        Ok(p.clone())
    }
    fn retained(&self, app: &Workspace) -> Result<(), String> {
        require(
            self.previous
                .as_ref()
                .zip(app.image_view.as_ref())
                .is_some_and(|(p, v)| Arc::ptr_eq(p, &v.pixels)),
            "Failure/Stop replaced prior preview pixels",
        )
    }
    fn write_input(&self, bytes: &[u8]) -> Result<PathBuf, String> {
        require(bytes.len() < 1024, "Fixed EPS fixture bound")?;
        let path = self.owned.join("active.eps");
        fs::write(&path, bytes).map_err(|e| e.to_string())?;
        Ok(path)
    }
    fn complete(&mut self, app: &Workspace) -> Result<(), String> {
        self.check_invariants(app)?;
        let receipt = serde_json::json!({"status":"actual_eps_case_completed_native_frame_pending", "case":format!("{:?}",self.config.case),
            "events":self.events,"renderer":app.eps_preview.renderer,"session_preview":app.preview_checkpoint(),
            "workspace_figure_document_model_invariants":true,"os_picker_or_input_claim":false,
            "preview_size":app.image_view.as_ref().map(|v|v.preview_size),
            "center_rgba":app.image_view.as_ref().map(|v| { let [w,h]=v.pixels.size;v.pixels.pixels[(h/2)*w+w/2].to_array() }),
            "dormant_frames":self.dormant_frames});
        let bytes = serde_json::to_vec_pretty(&receipt).map_err(|e| e.to_string())?;
        require(bytes.len() <= 65536, "EPS receipt bound")?;
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(self.owned.join("receipt.json"))
            .and_then(|mut f| f.write_all(&bytes))
            .map_err(|e| e.to_string())?;
        self.ready = true;
        Ok(())
    }
    fn advance(&mut self, app: &mut Workspace, ctx: &egui::Context) -> Result<(), String> {
        if self.ready {
            return Ok(());
        }
        require(
            self.started.elapsed() < Duration::from_secs(50),
            "EPS capture exceeded50 seconds",
        )?;
        if ctx.will_discard() || self.frame == Some(ctx.cumulative_frame_nr()) {
            return Ok(());
        }
        self.frame = Some(ctx.cumulative_frame_nr());
        let idle = !app.busy
            && app.pending_image.is_none()
            && app.open_queue.is_empty()
            && !app.recovering_session;
        if self.stage == 0 {
            if !idle || app.folder_browser.loading {
                return Ok(());
            }
            let renderer = app
                .eps_preview
                .renderer
                .as_ref()
                .ok_or("Configure an actual external renderer before EPS capture")?;
            let canonical = fs::canonicalize(renderer).map_err(|e| e.to_string())?;
            let repository =
                fs::canonicalize(env!("CARGO_MANIFEST_DIR")).map_err(|e| e.to_string())?;
            require(
                !canonical.starts_with(repository) && canonical.is_file(),
                "EPS acceptance renderer must be external to repository/tools",
            )?;
            require(
                !app.variables.is_empty()
                    && !app.all_figures.is_empty()
                    && !app.documents.is_empty(),
                "Seed the private acceptance session with actual workspace, figure and script documents",
            )?;
            self.baseline = Some(app.snapshot());
            self.figures = app.all_figures.clone();
            self.current = app.current_figure;
            self.history = app.figure_history.clone();
            self.documents = app.figure_documents.clone();
            if matches!(self.config.case, Case::Dormant | Case::Reopen) {
                require(
                    app.preview_restore.as_ref().is_some_and(|r| {
                        !r.started && r.error.is_none() && eps_preview::is_eps(&r.state.path)
                    }) && app.image_view.is_none(),
                    "Expected actual dormant recovered EPS reference",
                )?;
                return self.transition(20, "recovered dormant reference", app);
            }
            let path = self.write_input(RED)?;
            app.queue_open_paths([path]);
            return self.transition(1, "ordinary file-open queue", app);
        }
        match self.stage {
            1 if idle => {
                self.previous = Some(self.pixels(app, egui::Color32::RED)?);
                if self.config.case == Case::Open {
                    return self.complete(app);
                }
                let path = app.image_view.as_ref().unwrap().path.clone();
                app.eps_preview.configure_from_view(ctx, &path, true, true);
                self.transition(2, "open actual renderer settings", app)?;
            }
            2 if idle => {
                require(app.eps_preview.pending.is_some(), "Settings did not open")?;
                if self.config.case == Case::Settings {
                    return self.complete(app);
                }
                let path = app.image_view.as_ref().unwrap().path.clone();
                app.capture_eps_action(ctx, &path, eps_preview::Action::Cancel);
                require(
                    app.eps_preview.pending.is_none() && !app.busy,
                    "Settings Cancel submitted work",
                )?;
                self.retained(app)?;
                self.transition(9, "settings cancelled without worker", app)?;
            }
            9 if idle => {
                let path = app.image_view.as_ref().unwrap().path.clone();
                app.eps_preview.configure_from_view(ctx, &path, true, true);
                let configured = app.eps_preview.renderer.clone().ok_or("Missing renderer")?;
                app.select_eps_renderer(configured);
                self.transition(10, "configured existing external choice; no OS picker", app)?;
            }
            10 if idle => {
                let path = app
                    .eps_preview
                    .pending
                    .clone()
                    .ok_or("Missing settings path")?;
                app.capture_eps_action(ctx, &path, eps_preview::Action::Render);
                require(app.busy, "Settings Open preview was not admitted")?;
                self.transition(11, "actual settings Open preview action", app)?;
            }
            11 if idle => {
                self.pixels(app, egui::Color32::RED)?;
                let path = self.write_input(BLUE)?;
                // Same public worker route as the image-view Reload button.
                app.begin_eps_preview(path);
                self.transition(3, "actual reload after private source replacement", app)?;
            }
            3 if idle => {
                self.previous = Some(self.pixels(app, egui::Color32::BLUE)?);
                let path = self.write_input(INVALID)?;
                app.begin_eps_preview(path);
                self.transition(4, "actual interpreter failure request", app)?;
            }
            4 if idle => {
                self.retained(app)?;
                require(
                    app.eps_preview
                        .error
                        .as_ref()
                        .is_some_and(|s| !s.is_empty()),
                    "Interpreter failure was not observed",
                )?;
                let path = self.write_input(LOOP)?;
                app.capture_eps_action(ctx, &path, eps_preview::Action::Cancel);
                app.begin_eps_preview(path);
                require(app.busy && app.image_loading, "Loop job was not admitted")?;
                self.transition(5, "loop worker admitted", app)?;
            }
            5 if self.stage_started.elapsed() >= Duration::from_millis(250) => {
                require(
                    app.busy && app.image_loading,
                    "Loop finished before Stop acceptance",
                )?;
                app.stop_work();
                self.transition(6, "production Stop requested after admission", app)?;
            }
            6 if idle => {
                self.retained(app)?;
                require(
                    app.worker.cancel.load(Ordering::Relaxed),
                    "Stop flag lost before completion",
                )?;
                let path = self.write_input(LOOP)?;
                app.begin_eps_preview(path);
                require(app.busy, "Second loop not admitted")?;
                app.close_image_preview();
                self.transition(7, "close during admitted preview", app)?;
            }
            7 if idle => {
                require(
                    app.image_view.is_none() && app.pending_image.is_none(),
                    "Closed preview was republished",
                )?;
                let path = self.write_input(RED)?;
                app.begin_eps_preview(path);
                self.transition(8, "retry after cancellation and close", app)?;
            }
            8 if idle => {
                self.pixels(app, egui::Color32::RED)?;
                self.complete(app)?;
            }
            20 if idle => {
                require(
                    app.preview_restore.as_ref().is_some_and(|r| !r.started)
                        && app.image_view.is_none(),
                    "Recovery interpreted before explicit action",
                )?;
                self.dormant_frames += 1;
                if self.dormant_frames >= 3 {
                    if self.config.case == Case::Dormant {
                        return self.complete(app);
                    }
                    let path = app.preview_restore.as_ref().unwrap().state.path.clone();
                    require(
                        path.starts_with(&self.config.directory),
                        "Recovered EPS must remain private",
                    )?;
                    app.capture_eps_action(ctx, &path, eps_preview::Action::Reopen);
                    require(app.busy, "Explicit recovery reopen not admitted")?;
                    self.transition(21, "actual Reopen EPS action", app)?;
                }
            }
            21 if idle => {
                self.pixels(app, egui::Color32::RED)?;
                self.complete(app)?;
            }
            _ => {}
        }
        Ok(())
    }
}
impl Workspace {
    pub(super) fn eps_capture_frame(&mut self, ctx: &egui::Context) {
        let Some(mut capture) = self.eps_capture.take() else {
            return;
        };
        if let Err(error) = capture.advance(self, ctx) {
            let message = format!("EPS native capture failed: {error}");
            eprintln!("{message}");
            let _ = fs::write(capture.owned.join("failure.txt"), message);
            self.stop_work();
            self.allow_close = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else if !capture.ready() {
            ctx.request_repaint_after(Duration::from_millis(25));
        }
        self.eps_capture = Some(capture);
    }
}
#[cfg(test)]
#[path = "eps_capture_tests.rs"]
mod tests;
