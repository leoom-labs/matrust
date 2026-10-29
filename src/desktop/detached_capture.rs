//! Opt-in evidence collection from actual deferred native framebuffer replies.
//! Capture shutdown is separate from document Close and proves no OS gesture.
use super::*;
use std::path::{Path, PathBuf};

const MAX_PIXELS: usize = 4096 * 4096;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Token {
    nonce: u64,
    number: u32,
    revision: u64,
}
pub(super) struct Request {
    token: Token,
    key: Key,
    viewport: egui::ViewportId,
    ready: u8,
    last_frame: Option<u64>,
    requested: bool,
    reply: Option<matrust::Result<Arc<egui::ColorImage>>>,
    scale: f32,
}
fn image_reply(
    ctx: &egui::Context,
    viewport: egui::ViewportId,
    token: Token,
    requested: bool,
) -> Option<matrust::Result<Arc<egui::ColorImage>>> {
    ctx.input(|input| {
        let mut answer = None;
        for event in &input.events {
            if let egui::Event::Screenshot {
                viewport_id,
                user_data,
                image,
            } = event
            {
                if answer.is_some() {
                    return Some(Err("Duplicate native screenshot reply".into()));
                }
                let valid = user_data
                    .data
                    .as_ref()
                    .and_then(|data| data.downcast_ref::<Token>())
                    == Some(&token);
                answer = Some(if !requested || *viewport_id != viewport || !valid {
                    Err("Unexpected native screenshot viewport or request token".into())
                } else if image.width() == 0
                    || image.height() == 0
                    || image.width() > 4096
                    || image.height() > 4096
                    || image
                        .width()
                        .checked_mul(image.height())
                        .is_none_or(|n| n > MAX_PIXELS || n != image.pixels.len())
                {
                    Err("Native screenshot exceeds bounded dimensions".into())
                } else {
                    Ok(image.clone())
                });
            }
        }
        answer
    })
}
#[derive(Default)]
pub(super) struct FrameRequests {
    screenshot: Option<Token>,
    repaint: bool,
    wake_root: bool,
}
impl FrameRequests {
    pub(super) fn dispatch(self, ctx: &egui::Context) {
        if let Some(token) = self.screenshot {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(
                token,
            )));
        }
        if self.repaint {
            ctx.request_repaint();
        }
        if self.wake_root {
            ctx.request_repaint_of(egui::ViewportId::ROOT);
        }
    }
}
pub(super) fn child_frame(
    child: &mut Child,
    ctx: &egui::Context,
    class: egui::ViewportClass,
) -> FrameRequests {
    let mut commands = FrameRequests::default();
    let Some(request) = &mut child.capture else {
        return commands;
    };
    let frame = ctx.cumulative_frame_nr();
    if ctx.will_discard() || request.last_frame == Some(frame) {
        return commands;
    }
    request.last_frame = Some(frame);
    if class != egui::ViewportClass::Deferred
        || !child.key.same(&request.key)
        || child.presentation.revision != request.token.revision
        || child.viewport != request.viewport
    {
        request.reply = Some(Err(
            "Detached capture identity, revision or native viewport class changed".into(),
        ));
    } else if let Some(reply) = image_reply(ctx, request.viewport, request.token, request.requested)
    {
        request.reply = Some(if request.reply.is_some() {
            Err("Repeated detached screenshot reply".into())
        } else {
            reply
        });
    } else if !request.requested {
        if child.geometry.is_some()
            && !child.presentation.busy
            && !child.presentation.modal
            && !child.minimized
        {
            request.ready += 1;
            if request.ready >= 3 {
                request.requested = true;
                request.scale = ctx.pixels_per_point();
                commands.screenshot = Some(request.token);
            }
            // Only the finite ready-frame sequence (including screenshot
            // delivery) needs another child frame. Pending work/decisions are
            // refreshed by the root, preparation mailbox, or worker reply.
            commands.repaint = true;
        } else {
            request.ready = 0;
        }
    }
    commands.wake_root = request.reply.is_some();
    commands
}

pub(crate) struct Config {
    directory: PathBuf,
    root: PathBuf,
    compact: bool,
}
impl Config {
    pub(crate) fn parse(
        args: Vec<String>,
        options: &screenshot_options::Options,
        session: Option<PathBuf>,
    ) -> matrust::Result<Option<Self>> {
        let head: Vec<_> = args
            .iter()
            .take_while(|s| s.as_str() != "--open-files")
            .collect();
        if !head.iter().any(|s| s.as_str() == "--detached-capture") {
            return Ok(None);
        }
        if args.len() > 16 || args.iter().any(|a| a.len() > 4096) {
            return Err("Detached capture arguments exceed bounds".into());
        }
        let mut directory = None;
        let mut compact = false;
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--detached-capture"=>{
                    if directory.is_some(){return Err("Supply detached capture once".into());}
                    i+=1;let value=args.get(i).ok_or("Detached capture requires a fresh output directory")?;
                    if value.is_empty()||value.starts_with("--"){return Err("Invalid detached capture directory".into());}
                    directory=Some(PathBuf::from(value));
                }
                "--screenshot"|"--theme"|"--screenshot-frames"=>{i+=1;if i>=args.len(){return Err("Missing capture argument".into());}}
                "--compact-demo"=>compact=true,
                _=>return Err("Detached capture permits only screenshot/theme/size controls and a recovered private session".into()),
            }
            i += 1;
        }
        let directory = directory.unwrap();
        let root = options
            .path
            .clone()
            .ok_or("Detached capture requires --screenshot DIR/root.png")?;
        if !directory.is_absolute()
            || root != directory.join("root.png")
            || options.ready_frames != 3
            || options.theme.is_none()
        {
            return Err("Detached capture requires an absolute fresh directory, its root.png, and three ready frames".into());
        }
        if session
            .as_ref()
            .is_none_or(|path| !path.is_absolute() || !path.is_file())
        {
            return Err(
                "Detached capture requires an existing private MATRUST_SESSION_PATH checkpoint"
                    .into(),
            );
        }
        Ok(Some(Self {
            directory,
            root,
            compact,
        }))
    }
    pub(crate) fn start(self) -> matrust::Result<Coordinator> {
        std::fs::create_dir(&self.directory)
            .map_err(|e| format!("Fresh capture directory: {e}"))?;
        Ok(Coordinator {
            config: self,
            started: Instant::now(),
            targets: None,
            index: 0,
            records: Vec::new(),
            root_ready: 0,
            root_requested: false,
            last_frame: None,
            finished: false,
        })
    }
}
struct Target {
    key: Key,
    revision: u64,
    figure: Figure,
    viewport: egui::ViewportId,
}
pub(crate) struct Coordinator {
    config: Config,
    started: Instant,
    targets: Option<Vec<Target>>,
    index: usize,
    records: Vec<serde_json::Value>,
    root_ready: u8,
    root_requested: bool,
    last_frame: Option<u64>,
    finished: bool,
}
impl Coordinator {
    fn idle(app: &Workspace) -> bool {
        !app.busy
            && !app.file_action_pending()
            && app.open_queue.is_empty()
            && !app.folder_browser.loading
    }
    fn still_exact(&self, app: &Workspace) -> bool {
        self.targets.as_ref().is_none_or(|targets| {
            targets.len() == app.all_figures.len()
                && targets.iter().all(|target| {
                    live(app, &target.key)
                        && revision(app, target.key.number) == Some(target.revision)
                        && app.all_figures[&target.key.number].same_snapshot(&target.figure)
                })
        })
    }
    fn persist(
        &mut self,
        path: &Path,
        viewport: egui::ViewportId,
        token: Token,
        scale: f32,
        image: Arc<egui::ColorImage>,
    ) -> matrust::Result<()> {
        if path.exists() {
            return Err("Capture image path already exists".into());
        }
        save_screenshot(path, &image).map_err(|e| e.to_string())?;
        self.records.push(serde_json::json!({"file":path.file_name().unwrap().to_string_lossy(),"viewport":format!("{viewport:?}"),
            "figure":token.number,"revision":token.revision,"request_nonce":token.nonce,"ready_frames":3,
            "width":image.width(),"height":image.height(),"pixels_per_point":scale,"backend":"actual native framebuffer"}));
        Ok(())
    }
    fn advance(&mut self, app: &mut Workspace, ctx: &egui::Context) -> matrust::Result<bool> {
        if self.started.elapsed() > Duration::from_secs(45) {
            return Err("Detached native capture timed out".into());
        }
        if !self.still_exact(app) {
            return Err("Recovered figure identity/model changed during capture".into());
        }
        if self.targets.is_none() {
            if !Self::idle(app) {
                ctx.request_repaint();
                return Ok(false);
            }
            if app.all_figures.is_empty() || app.all_figures.len() > MAX_WINDOWS {
                return Err("Capture requires 1..16 genuinely recovered figures".into());
            }
            let ids: Vec<_> = app.all_figures.keys().copied().collect();
            for &id in &ids {
                app.undock_capture_figure(id, ctx);
            }
            let mut targets = Vec::new();
            for id in ids {
                let mut child = app
                    .detached
                    .windows
                    .get(&id)
                    .ok_or("Real Undock did not register expected figure")?
                    .lock()
                    .map_err(|_| "Detached viewport lock failed")?;
                child.capture_size = Some(if self.config.compact {
                    egui::vec2(640., 480.)
                } else {
                    egui::vec2(900., 650.)
                });
                targets.push(Target {
                    key: child.key.clone(),
                    revision: revision(app, id).ok_or("Missing document revision")?,
                    figure: app.all_figures[&id].clone(),
                    viewport: child.viewport,
                });
            }
            self.targets = Some(targets);
            ctx.request_repaint();
            return Ok(false);
        }
        let target_count = self.targets.as_ref().unwrap().len();
        if self.index < target_count {
            let target = &self.targets.as_ref().unwrap()[self.index];
            let shared = app
                .detached
                .windows
                .get(&target.key.number)
                .ok_or("Native figure disappeared")?
                .clone();
            let mut child = shared.lock().map_err(|_| "Detached viewport lock failed")?;
            if !child.key.same(&target.key) || child.viewport != target.viewport {
                return Err("Native viewport generation changed".into());
            }
            if child.capture.is_none() {
                child.prepare_requested |= child.geometry.is_none();
                child.capture = Some(Request {
                    token: Token {
                        nonce: self.index as u64 + 1,
                        number: target.key.number,
                        revision: target.revision,
                    },
                    key: target.key.clone(),
                    viewport: target.viewport,
                    ready: 0,
                    last_frame: None,
                    requested: false,
                    reply: None,
                    scale: 1.,
                });
            }
            let request = child.capture.as_mut().unwrap();
            let reply = request
                .reply
                .take()
                .map(|reply| (reply, request.token, request.scale));
            if reply.is_some() {
                child.capture = None;
            }
            let viewport = target.viewport;
            drop(child);
            if let Some((reply, token, scale)) = reply {
                let path = self
                    .config
                    .directory
                    .join(format!("figure-{}.png", token.number));
                self.persist(&path, viewport, token, scale, reply?)?;
                self.index += 1;
            } else {
                ctx.request_repaint_of(viewport);
            }
            ctx.request_repaint();
            return Ok(false);
        }
        if !Self::idle(app) {
            self.root_ready = 0;
            ctx.request_repaint();
            return Ok(false);
        }
        let token = Token {
            nonce: MAX_WINDOWS as u64 + 1,
            number: 0,
            revision: 0,
        };
        if let Some(reply) = image_reply(ctx, egui::ViewportId::ROOT, token, self.root_requested) {
            self.persist(
                &self.config.root.clone(),
                egui::ViewportId::ROOT,
                token,
                ctx.pixels_per_point(),
                reply?,
            )?;
            if self.records.len() != target_count + 1 || !self.still_exact(app) {
                return Err("Incomplete or changed capture set".into());
            }
            let result=serde_json::to_vec_pretty(&serde_json::json!({"format":1,"status":"captured_native_frames_pending_direct_review","theme":format!("{:?}",ctx.theme()),"images":&self.records,
                "limits":"Framebuffer evidence only; no OS title-bar X, Alt+F4, focus or chooser gesture acceptance"})).map_err(|e|e.to_string())?;
            if result.len() > 65536 {
                return Err("Capture receipt exceeds bound".into());
            }
            std::fs::write(self.config.directory.join("capture.json"), result)
                .map_err(|e| e.to_string())?;
            return Ok(true);
        }
        if !self.root_requested {
            self.root_ready += 1;
            if self.root_ready >= 3 {
                self.root_requested = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(
                    token,
                )));
            }
        }
        ctx.request_repaint();
        Ok(false)
    }
    pub(crate) fn frame(&mut self, app: &mut Workspace, ctx: &egui::Context) {
        let frame = ctx.cumulative_frame_nr();
        if self.finished || ctx.will_discard() || self.last_frame == Some(frame) {
            return;
        }
        self.last_frame = Some(frame);
        let done = match self.advance(app, ctx) {
            Ok(done) => done,
            Err(error) => {
                eprintln!("Detached capture failed: {error}");
                let _ = std::fs::write(self.config.directory.join("failure.txt"), &error);
                true
            }
        };
        if done {
            self.finished = true;
            // Test-process shutdown only: never claim this path tests window-X.
            let ids: Vec<_> = app.detached.windows.keys().copied().collect();
            for id in ids {
                app.detached.retire(id, ctx);
            }
            app.allow_close = true;
            ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Close);
        }
    }
}

#[cfg(test)]
#[path = "detached_capture_tests.rs"]
mod tests;
