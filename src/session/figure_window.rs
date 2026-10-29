//! Optional native-window checkpoint metadata and pure physical geometry.
//!
//! Recovery currently stages the containing manifest as serde_json::Value.
//! Bounds here constrain interpretation and accepted metadata, not that earlier
//! allocation. Normal Session loading retains its 32 MiB input/JSON depth caps.
use crate::Result;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub const MAX_MONITORS: usize = 32;
pub const MAX_WINDOW_BYTES: usize = 1024;
const MAX_COORDINATE: f64 = 1_000_000.;
const MAX_SPAN: f64 = 65536.;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NormalWindow {
    /// Content size in 96-DPI logical units, independent of UI zoom.
    pub inner_size_dip: [f64; 2],
    /// Physical desktop edges [left, top, right, bottom].
    pub outer_rect_px: [f64; 4],
    pub work_area_px: [f64; 4],
    pub monitor_device: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RawWindow")]
pub struct FigureWindow {
    version: u32,
    detached: bool,
    normal: Option<NormalWindow>,
    maximized: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawWindow {
    version: u32,
    detached: bool,
    normal: Option<NormalWindow>,
    maximized: bool,
}

impl TryFrom<RawWindow> for FigureWindow {
    type Error = String;
    fn try_from(raw: RawWindow) -> Result<Self> {
        if raw.version != 1 {
            return Err("Unsupported figure window version".into());
        }
        Self::new(raw.detached, raw.normal, raw.maximized)
    }
}

impl FigureWindow {
    pub fn new(detached: bool, normal: Option<NormalWindow>, maximized: bool) -> Result<Self> {
        if let Some(value) = &normal {
            validate_normal(value)?;
        }
        let value = Self {
            version: 1,
            detached,
            normal,
            maximized,
        };
        // Count the actual JSON representation without allocating a JSON buffer.
        struct Count(usize);
        impl std::io::Write for Count {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0 = self
                    .0
                    .checked_add(bytes.len())
                    .ok_or_else(|| std::io::Error::other("Window metadata size overflow"))?;
                if self.0 > MAX_WINDOW_BYTES {
                    return Err(std::io::Error::other("Window metadata exceeds 1 KiB"));
                }
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        serde_json::to_writer(Count(0), &value).map_err(|e| e.to_string())?;
        Ok(value)
    }
    pub fn detached(&self) -> bool {
        self.detached
    }
    pub fn normal(&self) -> Option<&NormalWindow> {
        self.normal.as_ref()
    }
    pub fn maximized(&self) -> bool {
        self.maximized
    }
}

fn valid_device(value: &str) -> bool {
    value.len() <= 128 && value.encode_utf16().count() <= 32 && !value.contains('\0')
}

fn validate_normal(value: &NormalWindow) -> Result<()> {
    if value
        .inner_size_dip
        .iter()
        .any(|v| !v.is_finite() || !(64. ..=16384.).contains(v))
    {
        return Err("Invalid figure window content size".into());
    }
    PhysicalRect::new(value.outer_rect_px)?;
    PhysicalRect::new(value.work_area_px)?;
    if value
        .monitor_device
        .as_deref()
        .is_some_and(|v| !valid_device(v))
    {
        return Err("Invalid figure window monitor hint".into());
    }
    Ok(())
}

/// A tolerant optional field. A reset is a layout notice, not data corruption.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WindowRecovery {
    value: Option<FigureWindow>,
    reset: bool,
}
impl WindowRecovery {
    pub fn admitted(value: FigureWindow) -> Self {
        Self {
            value: Some(value),
            reset: false,
        }
    }
    pub fn as_ref(&self) -> Option<&FigureWindow> {
        self.value.as_ref()
    }
    pub fn was_reset(&self) -> bool {
        self.reset
    }
    pub fn is_absent(&self) -> bool {
        self.value.is_none()
    }
}
impl Serialize for WindowRecovery {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        self.value.serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for WindowRecovery {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        if value.is_null() {
            return Ok(Self::default());
        }
        let mut remaining = 128;
        let object_shape = value.as_object().is_some_and(|object| {
            object
                .get("normal")
                .is_none_or(|normal| normal.is_null() || normal.is_object())
        });
        if object_shape
            && bounded_value(&value, 0, &mut remaining)
            && let Ok(value) = serde_json::from_value(value)
        {
            return Ok(Self::admitted(value));
        }
        Ok(Self {
            value: None,
            reset: true,
        })
    }
}
fn bounded_value(value: &serde_json::Value, depth: usize, remaining: &mut usize) -> bool {
    if depth > 8 || *remaining == 0 {
        return false;
    }
    *remaining -= 1;
    match value {
        serde_json::Value::Array(values) => {
            values.len() <= *remaining
                && values
                    .iter()
                    .all(|v| bounded_value(v, depth + 1, remaining))
        }
        serde_json::Value::Object(values) => {
            values.len() <= *remaining
                && values
                    .iter()
                    .all(|(k, v)| k.len() <= 128 && bounded_value(v, depth + 1, remaining))
        }
        serde_json::Value::String(value) => value.len() <= 256,
        _ => true,
    }
}

/// Validated physical desktop rectangle. Negative origins are supported.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PhysicalRect([f64; 4]);
impl PhysicalRect {
    pub fn new(edges: [f64; 4]) -> Result<Self> {
        if edges
            .iter()
            .any(|v| !v.is_finite() || v.abs() > MAX_COORDINATE)
            || !(0. ..=MAX_SPAN).contains(&(edges[2] - edges[0]))
            || !(0. ..=MAX_SPAN).contains(&(edges[3] - edges[1]))
            || edges[2] <= edges[0]
            || edges[3] <= edges[1]
        {
            return Err("Invalid physical window rectangle".into());
        }
        Ok(Self(edges))
    }
    pub fn edges(self) -> [f64; 4] {
        self.0
    }
    pub fn size(self) -> [f64; 2] {
        [self.0[2] - self.0[0], self.0[3] - self.0[1]]
    }
    pub fn contains(self, other: Self) -> bool {
        self.0[0] <= other.0[0]
            && self.0[1] <= other.0[1]
            && self.0[2] >= other.0[2]
            && self.0[3] >= other.0[3]
    }
    fn intersection(self, other: Self) -> f64 {
        (self.0[2].min(other.0[2]) - self.0[0].max(other.0[0])).max(0.)
            * (self.0[3].min(other.0[3]) - self.0[1].max(other.0[1])).max(0.)
    }
}

#[derive(Clone, Debug)]
pub struct MonitorArea {
    pub work: PhysicalRect,
    pub device: Option<String>,
    pub primary: bool,
}

/// Actual scale from the receiving child, never inferred from another window.
#[derive(Clone, Copy, Debug)]
pub struct ChildMetrics {
    pub native_pixels_per_point: f64,
    pub zoom: f64,
    /// Total outer-minus-inner physical decoration size on each axis.
    pub decoration_px: [f64; 2],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedWindow {
    pub monitor_index: usize,
    pub outer: PhysicalRect,
    pub inner_size_points: [f64; 2],
    pub outer_position_points: [f64; 2],
}

/// Select a current work area and resolve one correction using observed child
/// scale. The caller must re-observe after a cross-monitor move and resolve again.
/// This pure calculation does not prove the OS applied the returned commands.
pub fn resolve_window(
    saved: Option<&NormalWindow>,
    monitors: &[MonitorArea],
    root_outer: Option<PhysicalRect>,
    metrics: ChildMetrics,
    cascade: usize,
) -> Result<ResolvedWindow> {
    if monitors.is_empty()
        || monitors.len() > MAX_MONITORS
        || cascade >= 16
        || monitors
            .iter()
            .any(|m| m.device.as_deref().is_some_and(|s| !valid_device(s)))
        || !(0.25..=16.).contains(&metrics.native_pixels_per_point)
        || !(0.25..=8.).contains(&metrics.zoom)
        || metrics
            .decoration_px
            .iter()
            .any(|v| !v.is_finite() || !(0. ..=1024.).contains(v))
    {
        return Err("Window placement is unavailable".into());
    }
    if let Some(value) = saved {
        validate_normal(value)?;
    }
    let saved_outer = saved
        .map(|s| PhysicalRect::new(s.outer_rect_px))
        .transpose()?;
    let best_intersection = |rect: PhysicalRect| {
        let mut best = None;
        let mut area = 0.;
        for (index, monitor) in monitors.iter().enumerate() {
            let next = monitor.work.intersection(rect);
            if next > area {
                best = Some(index);
                area = next;
            }
        }
        best
    };
    let monitor_index = saved
        .and_then(|s| s.monitor_device.as_ref())
        .and_then(|name| {
            monitors
                .iter()
                .position(|m| m.device.as_ref() == Some(name))
        })
        .or_else(|| saved_outer.and_then(best_intersection))
        .or_else(|| root_outer.and_then(best_intersection))
        .or_else(|| monitors.iter().position(|m| m.primary))
        .ok_or("No current monitor can place the window")?;
    let work = monitors[monitor_index].work;
    let work_size = work.size();
    let native = metrics.native_pixels_per_point;
    let ppp = native * metrics.zoom;
    let wanted = saved.map_or([960., 640.], |s| s.inner_size_dip);
    let mut inner = [0.; 2];
    let mut outer_size = [0.; 2];
    let mut position = [0.; 2];
    for axis in 0..2 {
        let available = work_size[axis] - metrics.decoration_px[axis];
        if available < ppp {
            return Err("Monitor has no usable window content area".into());
        }
        // Effective ordinary minimum is reduced to fit genuinely small screens.
        let minimum = [440., 320.][axis] * ppp;
        inner[axis] = (wanted[axis] * native)
            .max(minimum.min(available))
            .min(available);
        outer_size[axis] = inner[axis] + metrics.decoration_px[axis];
        let travel = (work_size[axis] - outer_size[axis]).max(0.);
        let fraction = saved.and_then(|s| {
            let old_size = s.outer_rect_px[axis + 2] - s.outer_rect_px[axis];
            let old_travel = s.work_area_px[axis + 2] - s.work_area_px[axis] - old_size;
            (old_travel > 0.).then(|| {
                ((s.outer_rect_px[axis] - s.work_area_px[axis]) / old_travel).clamp(0., 1.)
            })
        });
        let offset = fraction.map_or_else(
            || (24. * (cascade + 1) as f64 * native).min(travel),
            |f| f * travel,
        );
        let upper = work.0[axis + 2] - outer_size[axis];
        if upper < work.0[axis] {
            return Err("Window rectangle cannot be represented safely".into());
        }
        position[axis] = (work.0[axis] + offset).clamp(work.0[axis], upper);
    }
    let outer = PhysicalRect::new([
        position[0],
        position[1],
        position[0] + outer_size[0],
        position[1] + outer_size[1],
    ])?;
    if !work.contains(outer) {
        return Err("Resolved window leaves its work area".into());
    }
    Ok(ResolvedWindow {
        monitor_index,
        outer,
        inner_size_points: [inner[0] / ppp, inner[1] / ppp],
        outer_position_points: [position[0] / ppp, position[1] / ppp],
    })
}
