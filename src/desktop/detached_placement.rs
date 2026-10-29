//! Placement metadata and bounded native correction, separate from document Close.
use super::*;
use matrust::session::figure_window::{
    ChildMetrics, FigureWindow, MonitorArea, NormalWindow, PhysicalRect, resolve_window,
};
use std::time::{Duration, Instant};
#[path = "window_monitors.rs"]
mod monitors;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Observation {
    inner: [f64; 4],
    outer: [f64; 4],
    native: f64,
    zoom: f64,
    minimized: bool,
    maximized: bool,
    fullscreen: bool,
    focused: bool,
}
impl Observation {
    fn metrics(self) -> Option<ChildMetrics> {
        let outer = PhysicalRect::new(self.outer).ok()?;
        let inner = PhysicalRect::new(self.inner).ok()?;
        if !outer.contains(inner)
            || !(0.25..=16.).contains(&self.native)
            || !(0.25..=8.).contains(&self.zoom)
        {
            return None;
        }
        let a = outer.size();
        let b = inner.size();
        Some(ChildMetrics {
            native_pixels_per_point: self.native,
            zoom: self.zoom,
            decoration_px: [a[0] - b[0], a[1] - b[1]],
        })
    }
    fn normal(self, monitor: &MonitorArea) -> Option<NormalWindow> {
        self.metrics()?;
        if self.minimized || self.maximized || self.fullscreen {
            return None;
        }
        let size = PhysicalRect::new(self.inner).ok()?.size();
        let round = |v: f64| v.round();
        let value = NormalWindow {
            inner_size_dip: [
                (size[0] / self.native * 16.).round() / 16.,
                (size[1] / self.native * 16.).round() / 16.,
            ],
            outer_rect_px: self.outer.map(round),
            work_area_px: monitor.work.edges(),
            monitor_device: monitor.device.clone(),
        };
        FigureWindow::new(true, Some(value.clone()), false).ok()?;
        Some(value)
    }
}
pub(super) fn observe(ctx: &egui::Context) -> Option<Observation> {
    let zoom = ctx.zoom_factor() as f64;
    ctx.input(|i| {
        let v = i.viewport();
        let native = v.native_pixels_per_point? as f64;
        let ppp = native * zoom;
        let edges = |r: egui::Rect| {
            [
                r.min.x as f64 * ppp,
                r.min.y as f64 * ppp,
                r.max.x as f64 * ppp,
                r.max.y as f64 * ppp,
            ]
        };
        let observation = Observation {
            inner: edges(v.inner_rect?),
            outer: edges(v.outer_rect?),
            native,
            zoom,
            minimized: v.minimized?,
            maximized: v.maximized?,
            fullscreen: v.fullscreen?,
            focused: v.focused.unwrap_or(false),
        };
        observation.metrics()?;
        Some(observation)
    })
}
pub(super) struct Message {
    pub viewport: egui::ViewportId,
    pub sequence: u64,
    pub observation: Observation,
}
struct State {
    key: Key,
    viewport: egui::ViewportId,
    saved: FigureWindow,
    restore: bool,
    started: Instant,
    sent: Option<(u64, Instant)>,
    attempts: u8,
    maximizing: bool,
    deferred_maximize: bool,
    seen_unfocused: bool,
    explicit_activation: bool,
    sequence: u64,
    observation: Option<Observation>,
    controls: Option<(u64, Option<monitors::Controls>)>,
}
#[derive(Default)]
pub(super) struct Manager {
    states: BTreeMap<u32, State>,
    attempted: Vec<Key>,
    topology: Option<Vec<MonitorArea>>,
    checked: Option<Instant>,
}
fn near(a: [f64; 4], b: [f64; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() <= 2.)
}
fn visible_controls(
    o: Observation,
    areas: &[MonitorArea],
    controls: monitors::Controls,
) -> Option<usize> {
    // Maximized outer bounds may include invisible resize-frame overhang.
    // Caption controls lie above the observed client top, within its horizontal
    // frame insets; do not require those invisible outer margins on-screen.
    let title = PhysicalRect::new([
        o.inner[2] - controls.minimum[0].min(o.inner[2] - o.inner[0]),
        (o.inner[1] - controls.title).max(o.outer[1]),
        o.inner[2],
        o.inner[1],
    ])
    .ok()?;
    areas.iter().position(|m| m.work.contains(title))
}
fn monitor_for(outer: [f64; 4], areas: &[MonitorArea]) -> Option<&MonitorArea> {
    let mut best = None;
    let mut area = 0.;
    for monitor in areas {
        let r = monitor.work.edges();
        let overlap = (r[2].min(outer[2]) - r[0].max(outer[0])).max(0.)
            * (r[3].min(outer[3]) - r[1].max(outer[1])).max(0.);
        if overlap > area {
            area = overlap;
            best = Some(monitor);
        }
    }
    best
}
enum Step {
    Wait,
    Correct(matrust::session::figure_window::ResolvedWindow),
    Maximize,
    Confirm,
    Dock,
}
impl State {
    fn step(
        &mut self,
        areas: &[MonitorArea],
        root: Option<PhysicalRect>,
        controls: Option<monitors::Controls>,
        now: Instant,
        cascade: usize,
    ) -> Step {
        if self.observation.is_some_and(|o| !o.focused) {
            self.seen_unfocused = true;
        }
        if !self.restore {
            if self.deferred_maximize
                && (self.explicit_activation || self.seen_unfocused)
                && self.observation.is_some_and(|o| o.focused)
            {
                self.restore = true;
                self.started = now;
                self.maximizing = true;
                self.deferred_maximize = false;
                return Step::Maximize;
            }
            return Step::Confirm;
        }
        if now.saturating_duration_since(self.started) >= Duration::from_secs(2) {
            return Step::Dock;
        }
        let Some(o) = self.observation else {
            return Step::Wait;
        };
        let Some(controls) = controls else {
            return Step::Dock;
        };
        if o.minimized || o.fullscreen {
            return Step::Dock;
        }
        if self.maximizing {
            if o.maximized && visible_controls(o, areas, controls).is_some() {
                self.restore = false;
                return Step::Confirm;
            }
            return Step::Wait;
        }
        if o.maximized {
            return Step::Dock;
        }
        let Some(metrics) = o.metrics() else {
            return Step::Dock;
        };
        let Ok(target) = resolve_window(self.saved.normal(), areas, root, metrics, cascade) else {
            return Step::Dock;
        };
        let size = target.outer.size();
        if size[0] < controls.minimum[0]
            || size[1] < controls.minimum[1]
            || size[1] <= controls.title
            || target
                .inner_size_points
                .iter()
                .any(|v| !(64. ..=16384.).contains(&(v * o.zoom)))
        {
            return Step::Dock;
        }
        // Confirm actual geometry, not merely a successfully emitted command.
        if self.sent.is_some_and(|(seq, _)| self.sequence > seq)
            && near(o.outer, target.outer.edges())
            && areas[target.monitor_index]
                .work
                .contains(PhysicalRect::new(o.outer).unwrap())
        {
            if let Some(normal) = o.normal(&areas[target.monitor_index])
                && let Ok(saved) = FigureWindow::new(true, Some(normal), self.saved.maximized())
            {
                self.saved = saved;
            }
            if self.saved.maximized() {
                // winit's later maximize command activates on Windows. Defer it
                // until observed focus. Background restore additionally needs a
                // preceding unfocused observation; explicit Undock does not.
                if o.focused && (self.explicit_activation || self.seen_unfocused) {
                    self.maximizing = true;
                    return Step::Maximize;
                }
                self.deferred_maximize = true;
            }
            self.restore = false;
            return Step::Confirm;
        }
        if self
            .sent
            .is_some_and(|(_, at)| now.saturating_duration_since(at) < Duration::from_millis(150))
        {
            return Step::Wait;
        }
        if self.attempts >= 3 {
            return Step::Wait;
        }
        self.attempts += 1;
        self.sent = Some((self.sequence, now));
        Step::Correct(target)
    }
}
pub(super) fn publish(app: &mut Workspace, key: &Key, window: FigureWindow) {
    if !live(app, key) {
        return;
    }
    if app
        .figure_windows
        .get(&key.number)
        .is_some_and(|p| p.identity.same_object(&key.identity) && p.window == window)
    {
        return;
    }
    app.figure_windows.insert(
        key.number,
        figure_recovery::Placement {
            identity: key.identity.clone(),
            window,
        },
    );
    app.recovery_dirty = true;
}
impl Manager {
    pub(super) fn created(
        &mut self,
        key: Key,
        viewport: egui::ViewportId,
        saved: Option<FigureWindow>,
        explicit_activation: bool,
    ) {
        let saved =
            saved.unwrap_or_else(|| FigureWindow::new(true, None, false).expect("bounded default"));
        self.attempted.retain(|k| k.number != key.number);
        self.attempted.push(key.clone());
        self.states.insert(
            key.number,
            State {
                key,
                viewport,
                saved,
                restore: true,
                started: Instant::now(),
                sent: None,
                attempts: 0,
                maximizing: false,
                deferred_maximize: false,
                seen_unfocused: false,
                explicit_activation,
                sequence: 0,
                observation: None,
                controls: None,
            },
        );
    }
    pub(super) fn accept(&mut self, key: &Key, message: Message) {
        if let Some(state) = self.states.get_mut(&key.number)
            && state.key.same(key)
            && state.viewport == message.viewport
            && message.sequence > state.sequence
            && message.observation.metrics().is_some()
        {
            state.sequence = message.sequence;
            state.observation = Some(message.observation);
        }
    }
    pub(super) fn docked(&mut self, app: &mut Workspace, key: &Key) {
        if self
            .states
            .get(&key.number)
            .is_some_and(|s| s.key.same(key))
        {
            let state = self
                .states
                .remove(&key.number)
                .expect("matched placement state");
            if let Ok(saved) = FigureWindow::new(
                false,
                state.saved.normal().cloned(),
                state.saved.maximized(),
            ) {
                publish(app, key, saved);
            }
        } else if let Some(old) = app
            .figure_windows
            .get(&key.number)
            .filter(|p| p.identity.same_object(&key.identity))
            && let Ok(saved) =
                FigureWindow::new(false, old.window.normal().cloned(), old.window.maximized())
        {
            publish(app, key, saved);
        }
    }
    fn checkpoint(&mut self, app: &mut Workspace, areas: &[MonitorArea]) {
        for state in self.states.values_mut() {
            if !live(app, &state.key) {
                continue;
            }
            let Some(o) = state.observation else {
                continue;
            };
            if state.restore || o.fullscreen {
                continue;
            }
            let normal = if !o.minimized && !o.maximized {
                monitor_for(o.outer, areas)
                    .and_then(|m| o.normal(m))
                    .or_else(|| {
                        // Final quit may have no fresh topology. Preserve the newest
                        // observed normal bounds using the last validated work-area
                        // hint; a future restore always reclamps to current topology.
                        let previous = state.saved.normal()?;
                        let monitor = MonitorArea {
                            work: PhysicalRect::new(previous.work_area_px).ok()?,
                            device: previous.monitor_device.clone(),
                            primary: false,
                        };
                        o.normal(&monitor)
                    })
                    .or_else(|| state.saved.normal().cloned())
            } else {
                state.saved.normal().cloned()
            };
            // Minimize preserves the last nonminimized mode and normal bounds.
            let maximized = if o.minimized || state.deferred_maximize {
                state.saved.maximized()
            } else {
                o.maximized
            };
            if let Ok(window) = FigureWindow::new(true, normal, maximized) {
                state.saved = window.clone();
                publish(app, &state.key, window);
            }
        }
    }
}
impl Registry {
    pub(super) fn flush_placement(&mut self, app: &mut Workspace) {
        if self.capture_mode || app.detached_capture.is_some() {
            return;
        }
        for child in self.windows.values() {
            if let Ok(mut child) = child.lock()
                && let Some(message) = child.mailbox.native.take()
            {
                self.placement.accept(&child.key, message);
            }
        }
        let areas = self.placement.topology.take().unwrap_or_default();
        self.placement.checkpoint(app, &areas);
        self.placement.topology = (!areas.is_empty()).then_some(areas);
    }
    pub(super) fn placement_frame(&mut self, app: &mut Workspace, ctx: &egui::Context) {
        // All native enumeration/metrics and framework commands happen WITHOUT
        // a Child mutex. Capture uses deliberately transient windows.
        if app.detached_capture.is_some() || self.capture_mode {
            return;
        }
        let mut manager = std::mem::take(&mut self.placement);
        manager
            .states
            .retain(|_, s| live(app, &s.key) && self.windows.contains_key(&s.key.number));
        manager.attempted.retain(|k| live(app, k));
        let now = Instant::now();
        let wanted = app.figure_windows.iter().any(|(&id, p)| {
            p.window.detached()
                && !self.windows.contains_key(&id)
                && !manager
                    .attempted
                    .iter()
                    .any(|k| k.number == id && k.identity.same_object(&p.identity))
        });
        if manager.states.is_empty() && !wanted {
            self.placement = manager;
            return;
        }
        let refreshed = manager
            .checked
            .is_none_or(|at| now.saturating_duration_since(at) >= Duration::from_secs(1));
        if refreshed {
            manager.topology = monitors::enumerate();
            manager.checked = Some(now);
        }
        let root = observe(ctx).and_then(|o| PhysicalRect::new(o.outer).ok());
        let areas = manager.topology.take().unwrap_or_default();
        let mut fallback = Vec::new();
        let blocked = app.busy || app.file_action_pending() || self.pending.is_some();
        for (cascade, (&id, state)) in manager.states.iter_mut().enumerate() {
            if blocked || app.allow_close {
                // Worker preparation and root decisions do not consume the
                // correction timeout; attempt count is never reset by them.
                if state.restore {
                    state.started = now;
                }
                continue;
            }
            let controls = state.observation.and_then(|o| {
                if refreshed
                    || state
                        .controls
                        .is_none_or(|(bits, _)| bits != o.native.to_bits())
                {
                    state.controls = Some((o.native.to_bits(), monitors::controls(o.native)));
                }
                state.controls.and_then(|(_, controls)| controls)
            });
            if !state.restore
                && !areas.is_empty()
                && let (Some(o), Some(c)) = (state.observation, controls)
                && !o.minimized
                && !o.fullscreen
                && visible_controls(o, &areas, c).is_none()
            {
                state.restore = true;
                state.started = now;
                state.sent = None;
                state.attempts = 0;
                state.maximizing = false;
            }
            let step = if areas.is_empty() {
                Step::Dock
            } else {
                state.step(&areas, root, controls, now, cascade)
            };
            match step {
                Step::Correct(target) => {
                    // A smaller minimum is applied only after checking actual OS
                    // minimum tracking/control metrics at the child's DPI.
                    let size = egui::vec2(
                        target.inner_size_points[0] as f32,
                        target.inner_size_points[1] as f32,
                    );
                    if let Some(child) = self.windows.get(&id)
                        && let Ok(mut child) = child.lock()
                    {
                        child.minimum_size = size.min(egui::vec2(440., 320.));
                    }
                    ctx.send_viewport_cmd_to(
                        state.viewport,
                        egui::ViewportCommand::MinInnerSize(size.min(egui::vec2(440., 320.))),
                    );
                    ctx.send_viewport_cmd_to(
                        state.viewport,
                        egui::ViewportCommand::InnerSize(size),
                    );
                    ctx.send_viewport_cmd_to(
                        state.viewport,
                        egui::ViewportCommand::OuterPosition(egui::pos2(
                            target.outer_position_points[0] as f32,
                            target.outer_position_points[1] as f32,
                        )),
                    );
                }
                Step::Maximize => {
                    ctx.send_viewport_cmd_to(state.viewport, egui::ViewportCommand::Maximized(true))
                }
                Step::Dock => fallback.push(state.key.clone()),
                Step::Confirm | Step::Wait => {}
            }
            if let Some(child) = self.windows.get(&id)
                && let Ok(mut child) = child.lock()
            {
                child.placement_pending = state.restore;
            }
            if state.restore {
                ctx.request_repaint_after_for(Duration::from_millis(100), state.viewport);
                ctx.request_repaint_after(Duration::from_millis(100));
            }
        }
        manager.checkpoint(app, &areas);
        manager.topology = (!areas.is_empty()).then_some(areas);
        for key in fallback {
            manager.docked(app, &key);
            // An inactive recovery failure needs no selection or root canvas
            // transfer. Preserve the admitted current figure and focus. For the
            // current figure retain real Dock preparation before presentation.
            if app.current_figure != key.number {
                self.retire(key.number, ctx);
            } else if let Some(child) = self.windows.get(&key.number)
                && let Ok(mut child) = child.lock()
            {
                child.placement_pending = false;
                child.mailbox.command = Some(Envelope {
                    key: key.clone(),
                    revision: child.presentation.revision,
                    sequence: next(&self.sequence),
                    command: Command::Action(Action::Dock),
                });
            }
            app.status =
                "Figure placement could not be confirmed; returning it to the desktop.".into();
            ctx.request_repaint();
        }
        let pending = if !app.allow_close && !blocked {
            app.figure_windows.iter().find_map(|(&id, p)| {
                let key = Key {
                    number: id,
                    identity: p.identity.clone(),
                };
                (p.window.detached()
                    && live(app, &key)
                    && !self.windows.contains_key(&id)
                    && !manager.attempted.iter().any(|k| k.same(&key)))
                .then_some(id)
            })
        } else {
            None
        };
        self.placement = manager;
        if let Some(id) = pending {
            if self.placement.topology.is_some() {
                self.create_window(app, id, ctx, false);
            } else if let Some(p) = app.figure_windows.get(&id).cloned() {
                let key = Key {
                    number: id,
                    identity: p.identity,
                };
                self.placement.attempted.push(key.clone());
                if let Ok(window) =
                    FigureWindow::new(false, p.window.normal().cloned(), p.window.maximized())
                {
                    publish(app, &key, window);
                }
                app.status =
                    "Saved figure remains docked because monitor placement is unavailable.".into();
            }
        }
        if !self.windows.is_empty() {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
    }
}

#[cfg(test)]
#[path = "detached_placement_tests.rs"]
mod tests;

impl Workspace {
    pub(crate) fn flush_detached_placement(&mut self) {
        let mut registry = std::mem::take(&mut self.detached);
        registry.flush_placement(self);
        self.detached = registry;
    }
}
