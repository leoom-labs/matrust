//! Control System Designer: shapes a compensator C for a SISO loop on root
//! locus, Bode and Nichols editors and shows the closed-loop step. The loop
//! has a plant G, a sensor H and a prefilter F (workspace models or typed
//! transfer functions); C sits in the forward path or in the feedback path.
//! C = K times monic zpk factors, one or two per element, so the root locus
//! gain is K. Every number shown comes from engine builtins (`rlocus`,
//! `bode`, `margin`, `nichols`, `ngrid`, `step`, `stepinfo`, `damp`, `pole`,
//! `zero`, `isstable`); the app only maps its parameters to zpk roots.
use super::pid_tuner::parse_coefficients;
use super::{App, AppKind, Outputs, Snapshot, field, name_field, number, short, unused_name};
use eframe::egui;
use matrust::{Array, Complex64 as Z};
use std::f64::consts::PI;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// Most dynamic elements in C.
pub const MAX_ELEMENTS: usize = 32;
/// Most undo steps kept.
const MAX_HISTORY: usize = 200;
/// Settling time of a pole region: Ts = SETTLING_SIGMA / sigma (2 % band).
pub const SETTLING_SIGMA: f64 = 4.;
/// A gain drag never takes C's gain below this fraction of its start value.
const MIN_FACTOR: f64 = 1e-6;
/// Pick radius of draggable markers, in pixels.
const HIT: f32 = 10.;
/// Largest damping ratio a complex pair keeps (1 is a double real root).
const MAX_ZETA: f64 = 0.999;

// ---------------------------------------------------------------------------
// Loop

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Architecture {
    /// r -> F -> (+) -> C -> G -> y, H in the feedback path.
    Forward,
    /// r -> F -> (+) -> G -> y, C and H in the feedback path.
    Feedback,
}
impl Architecture {
    pub const ALL: [Self; 2] = [Self::Forward, Self::Feedback];
    pub fn label(self) -> &'static str {
        match self {
            Self::Forward => "C in the forward path",
            Self::Feedback => "C in the feedback path",
        }
    }
    pub fn formula(self) -> &'static str {
        match self {
            Self::Forward => "T = F·C·G / (1 + C·G·H)",
            Self::Feedback => "T = F·G / (1 + C·G·H)",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Plant,
    Sensor,
    Prefilter,
}
impl Role {
    pub const ALL: [Self; 3] = [Self::Plant, Self::Sensor, Self::Prefilter];
    pub fn label(self) -> &'static str {
        match self {
            Self::Plant => "Plant G",
            Self::Sensor => "Sensor H",
            Self::Prefilter => "Prefilter F",
        }
    }
    fn base(self) -> &'static str {
        match self {
            Self::Plant => "G",
            Self::Sensor => "H",
            Self::Prefilter => "F",
        }
    }
}

/// Where a model comes from: a workspace tf/zpk/ss variable or typed
/// coefficients of a transfer function.
#[derive(Clone, Debug, PartialEq)]
pub struct Source {
    pub use_variable: bool,
    pub variable: String,
    pub numerator: String,
    pub denominator: String,
}
impl Source {
    pub fn typed(numerator: &str, denominator: &str) -> Self {
        Self {
            use_variable: false,
            variable: String::new(),
            numerator: numerator.into(),
            denominator: denominator.into(),
        }
    }
    #[cfg(test)]
    pub fn variable(name: &str) -> Self {
        Self {
            use_variable: true,
            variable: name.into(),
            ..Self::typed("1", "1")
        }
    }
}

/// Sample time of a workspace LTI model; 0 is continuous time.
pub fn model_ts(array: &Array) -> Option<f64> {
    matrust::engine::lti_class(array)?;
    let Array::Struct(s) = array else {
        return None;
    };
    match s.get(s.field_index("Ts")?, 0).ok()?.as_ref() {
        Array::Numeric(m) => m.data().first().copied(),
        _ => None,
    }
}

/// The resolved loop of one script: expressions of G, H and F (a name or
/// `1`), the lines defining typed models, and the output names.
#[derive(Clone, Debug, PartialEq)]
pub struct Setup {
    pub g: String,
    pub h: String,
    pub f: String,
    pub definitions: Vec<String>,
    /// Names the definitions create.
    pub defined: Vec<String>,
    pub c: String,
    pub t: String,
    /// Sample time of the loop (G's); 0 is continuous time.
    pub ts: f64,
}

/// `a*b*...` without unit factors.
fn product(parts: &[&str]) -> String {
    let kept: Vec<&str> = parts.iter().copied().filter(|p| *p != "1").collect();
    if kept.is_empty() {
        "1".into()
    } else {
        kept.join("*")
    }
}
fn prefiltered(f: &str, inner: String) -> String {
    if f == "1" {
        inner
    } else {
        format!("{f}*{inner}")
    }
}
impl Setup {
    /// C*G*H.
    pub fn open_loop(&self) -> String {
        product(&[&self.c, &self.g, &self.h])
    }
    /// The closed loop from r to y.
    pub fn closed_loop(&self, architecture: Architecture) -> String {
        prefiltered(
            &self.f,
            match architecture {
                Architecture::Forward => {
                    format!("feedback({}, {})", product(&[&self.c, &self.g]), self.h)
                }
                Architecture::Feedback => {
                    format!("feedback({}, {})", self.g, product(&[&self.c, &self.h]))
                }
            },
        )
    }
    /// The closed loop from r to the plant input u.
    pub fn effort(&self, architecture: Architecture) -> String {
        prefiltered(
            &self.f,
            match architecture {
                Architecture::Forward => {
                    format!("feedback({}, {})", self.c, product(&[&self.g, &self.h]))
                }
                Architecture::Feedback => format!("feedback(1, {})", self.open_loop()),
            },
        )
    }
    /// The same loop with the probe's private names.
    fn private(&self) -> Self {
        let name = |expr: &str, private: &str| {
            if expr == "1" {
                "1".to_owned()
            } else {
                private.to_owned()
            }
        };
        Self {
            g: "app_G".into(),
            h: name(&self.h, "app_H"),
            f: name(&self.f, "app_F"),
            definitions: Vec::new(),
            defined: Vec::new(),
            c: "app_C".into(),
            t: "app_T".into(),
            ts: self.ts,
        }
    }
}

// ---------------------------------------------------------------------------
// Compensator

/// s = -zeta*wn + j*wn*sqrt(1 - zeta^2), the upper root of s^2 + 2 zeta wn s + wn^2.
fn upper_root(wn: f64, zeta: f64) -> Z {
    Z::new(-zeta * wn, wn * (1. - zeta * zeta).max(0.).sqrt())
}
/// A point of the s-plane in the loop's plane (z = e^(s Ts) when discrete).
pub fn to_plane(s: Z, ts: f64) -> Z {
    if ts > 0. { (s * ts).exp() } else { s }
}
/// A point of the loop's plane in the s-plane (s = ln(z)/Ts when discrete).
pub fn to_s(p: Z, ts: f64) -> Z {
    if ts > 0. { p.ln() / ts } else { p }
}
/// Natural frequency and damping ratio of a root, as `damp` defines them;
/// a root at s = 0 has damping -1 and z = 0 (s = -Inf) damping 1.
pub fn natural(p: Z, ts: f64) -> (f64, f64) {
    let s = to_s(p, ts);
    let wn = s.norm();
    let zeta = if wn.is_infinite() {
        1.
    } else if wn > 0. {
        -s.re / wn
    } else {
        -1.
    };
    (wn, zeta)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ElementKind {
    RealPole,
    RealZero,
    ComplexPoles,
    ComplexZeros,
    Integrator,
    Differentiator,
    Lead,
    Lag,
    Notch,
}
impl ElementKind {
    pub const ALL: [Self; 9] = [
        Self::RealPole,
        Self::RealZero,
        Self::ComplexPoles,
        Self::ComplexZeros,
        Self::Integrator,
        Self::Differentiator,
        Self::Lead,
        Self::Lag,
        Self::Notch,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::RealPole => "Real pole",
            Self::RealZero => "Real zero",
            Self::ComplexPoles => "Complex pole pair",
            Self::ComplexZeros => "Complex zero pair",
            Self::Integrator => "Integrator",
            Self::Differentiator => "Differentiator",
            Self::Lead => "Lead",
            Self::Lag => "Lag",
            Self::Notch => "Notch",
        }
    }
}

/// One dynamic element of C. Real roots are locations in the loop's plane
/// (s, or z when discrete); pairs are natural frequency (rad/s) and damping,
/// placed at z = e^(s Ts) when discrete. A notch is a zero pair (damping
/// `depth`) over a pole pair (damping `width`) at one frequency.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Element {
    RealPole(f64),
    RealZero(f64),
    ComplexPoles { wn: f64, zeta: f64 },
    ComplexZeros { wn: f64, zeta: f64 },
    Integrator,
    Differentiator,
    Lead { zero: f64, pole: f64 },
    Lag { zero: f64, pole: f64 },
    Notch { wn: f64, depth: f64, width: f64 },
}
/// The pole or zero group of one element that a drag moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Handle {
    pub element: usize,
    pub zero: bool,
}
/// One root of C as drawn; fixed roots (integrator, differentiator) have no handle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Marker {
    pub handle: Option<Handle>,
    pub root: Z,
    pub zero: bool,
}

/// A real root at `s` (rad/s) in the loop's plane.
fn real_root(s: f64, ts: f64) -> f64 {
    to_plane(Z::new(s, 0.), ts).re
}
fn finite(values: &[f64]) -> bool {
    values.iter().all(|v| v.is_finite())
}

impl Element {
    pub fn kind(&self) -> ElementKind {
        match self {
            Self::RealPole(_) => ElementKind::RealPole,
            Self::RealZero(_) => ElementKind::RealZero,
            Self::ComplexPoles { .. } => ElementKind::ComplexPoles,
            Self::ComplexZeros { .. } => ElementKind::ComplexZeros,
            Self::Integrator => ElementKind::Integrator,
            Self::Differentiator => ElementKind::Differentiator,
            Self::Lead { .. } => ElementKind::Lead,
            Self::Lag { .. } => ElementKind::Lag,
            Self::Notch { .. } => ElementKind::Notch,
        }
    }
    /// A new element with its default parameters, placed in the loop's plane.
    pub fn new(kind: ElementKind, ts: f64) -> Self {
        let real = |s| real_root(s, ts);
        match kind {
            ElementKind::RealPole => Self::RealPole(real(-10.)),
            ElementKind::RealZero => Self::RealZero(real(-1.)),
            ElementKind::ComplexPoles => Self::ComplexPoles { wn: 10., zeta: 0.7 },
            ElementKind::ComplexZeros => Self::ComplexZeros { wn: 1., zeta: 0.7 },
            ElementKind::Integrator => Self::Integrator,
            ElementKind::Differentiator => Self::Differentiator,
            ElementKind::Lead => Self::Lead {
                zero: real(-1.),
                pole: real(-10.),
            },
            ElementKind::Lag => Self::Lag {
                zero: real(-0.1),
                pole: real(-0.01),
            },
            ElementKind::Notch => Self::Notch {
                wn: 5.,
                depth: 0.05,
                width: 0.5,
            },
        }
    }
    /// Zeros and poles of the element's monic factor in the loop's plane.
    pub fn roots(&self, ts: f64) -> (Vec<Z>, Vec<Z>) {
        let origin = Z::new(if ts > 0. { 1. } else { 0. }, 0.);
        let pair = |wn: f64, zeta: f64| {
            let r = to_plane(upper_root(wn, zeta), ts);
            vec![r, r.conj()]
        };
        let real = |x: f64| vec![Z::new(x, 0.)];
        match *self {
            Self::RealPole(p) => (vec![], real(p)),
            Self::RealZero(z) => (real(z), vec![]),
            Self::ComplexPoles { wn, zeta } => (vec![], pair(wn, zeta)),
            Self::ComplexZeros { wn, zeta } => (pair(wn, zeta), vec![]),
            Self::Integrator => (vec![], vec![origin]),
            Self::Differentiator => (vec![origin], vec![]),
            Self::Lead { zero, pole } | Self::Lag { zero, pole } => (real(zero), real(pole)),
            Self::Notch { wn, depth, width } => (pair(wn, depth), pair(wn, width)),
        }
    }
    /// What is wrong with the parameters.
    pub fn check(&self) -> Result<(), String> {
        let pair = |wn: f64, zeta: f64| {
            if !(wn > 0. && wn.is_finite()) {
                Err("the natural frequency must be a positive number".to_owned())
            } else if zeta.is_nan() || zeta.abs() > MAX_ZETA {
                Err(format!(
                    "the damping must be between -{MAX_ZETA} and {MAX_ZETA}"
                ))
            } else {
                Ok(())
            }
        };
        match *self {
            Self::RealPole(x) | Self::RealZero(x) if !x.is_finite() => {
                Err("the location must be a number".into())
            }
            Self::Lead { zero, pole } | Self::Lag { zero, pole } if !finite(&[zero, pole]) => {
                Err("the zero and pole must be numbers".into())
            }
            Self::ComplexPoles { wn, zeta } | Self::ComplexZeros { wn, zeta } => pair(wn, zeta),
            Self::Notch { wn, depth, width } => {
                pair(wn, depth)?;
                pair(wn, width)?;
                if depth < 0. || width <= 0. {
                    Err("the notch dampings must be positive".into())
                } else {
                    Ok(())
                }
            }
            _ => Ok(()),
        }
    }
    /// The element's factor as a zpk call with gain 1.
    pub fn factor(&self, ts: f64) -> String {
        let (zeros, poles) = self.roots(ts);
        zpk_call(&zeros, &poles, 1., ts)
    }
    /// Its parameters in words, for the script comment.
    pub fn describe(&self) -> String {
        let kind = self.kind().label().to_lowercase();
        match *self {
            Self::RealPole(x) | Self::RealZero(x) => format!("{kind} at {}", short(x)),
            Self::ComplexPoles { wn, zeta } | Self::ComplexZeros { wn, zeta } => {
                format!("{kind}, wn {} rad/s, damping {}", short(wn), short(zeta))
            }
            Self::Integrator | Self::Differentiator => kind,
            Self::Lead { zero, pole } | Self::Lag { zero, pole } => {
                format!("{kind}, zero at {}, pole at {}", short(zero), short(pole))
            }
            Self::Notch { wn, depth, width } => format!(
                "{kind} at {} rad/s, zero damping {}, pole damping {}",
                short(wn),
                short(depth),
                short(width)
            ),
        }
    }
    fn movable(&self) -> bool {
        !matches!(self, Self::Integrator | Self::Differentiator)
    }
}

/// A complex number written so the engine reads back the same doubles.
fn complex(z: Z) -> String {
    if z.im == 0. {
        number(z.re)
    } else {
        format!(
            "{}{}{}i",
            number(z.re),
            if z.im < 0. { "-" } else { "+" },
            number(z.im.abs())
        )
    }
}
fn roots_vector(roots: &[Z]) -> String {
    match roots {
        [] => "[]".into(),
        [one] => complex(*one),
        _ => format!(
            "[{}]",
            roots
                .iter()
                .map(|r| complex(*r))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}
fn zpk_call(zeros: &[Z], poles: &[Z], gain: f64, ts: f64) -> String {
    let ts = if ts > 0. {
        format!(", {}", number(ts))
    } else {
        String::new()
    };
    format!(
        "zpk({}, {}, {}{ts})",
        roots_vector(zeros),
        roots_vector(poles),
        number(gain)
    )
}

/// Factors of a monic polynomial with these roots, as text: `s`, `(s + 2)`,
/// `(s^2 + 1.4 s + 1)` for a complex pair.
fn factors(roots: &[Z], v: &str) -> Vec<String> {
    let mut out = Vec::new();
    for r in roots {
        if r.im < 0. {
            continue;
        }
        if r.im > 0. {
            let b = -2. * r.re;
            let c = r.norm_sqr();
            let middle = if b == 0. {
                String::new()
            } else {
                format!(" {} {} {v}", if b < 0. { "-" } else { "+" }, short(b.abs()))
            };
            out.push(format!("({v}^2{middle} + {})", short(c)));
        } else if r.re == 0. {
            out.push(v.to_owned());
        } else {
            out.push(format!(
                "({v} {} {})",
                if r.re < 0. { "+" } else { "-" },
                short(r.re.abs())
            ));
        }
    }
    out
}

/// C = gain times the elements' monic factors.
#[derive(Clone, Debug, PartialEq)]
pub struct Compensator {
    pub gain: f64,
    pub elements: Vec<Element>,
}
impl Default for Compensator {
    fn default() -> Self {
        Self {
            gain: 1.,
            elements: Vec::new(),
        }
    }
}
impl Compensator {
    pub fn roots(&self, ts: f64) -> (Vec<Z>, Vec<Z>) {
        let (mut zeros, mut poles) = (Vec::new(), Vec::new());
        for element in &self.elements {
            let (z, p) = element.roots(ts);
            zeros.extend(z);
            poles.extend(p);
        }
        (zeros, poles)
    }
    /// A gain, valid elements and no more zeros than poles.
    pub fn check(&self, ts: f64) -> Result<(), String> {
        if !(self.gain.is_finite() && self.gain != 0.) {
            return Err("Compensator: the gain must be a nonzero number".into());
        }
        if self.elements.len() > MAX_ELEMENTS {
            return Err(format!("Compensator: at most {MAX_ELEMENTS} elements"));
        }
        for (i, element) in self.elements.iter().enumerate() {
            element.check().map_err(|e| {
                format!(
                    "Compensator element {} ({}): {e}",
                    i + 1,
                    element.kind().label()
                )
            })?;
        }
        let (zeros, poles) = self.roots(ts);
        if zeros.len() > poles.len() {
            return Err(format!(
                "Compensator: C has {} zeros and {} poles, so it is improper. Add a pole \
                 (for example a real pole well left of the zeros) or remove a zero.",
                zeros.len(),
                poles.len()
            ));
        }
        Ok(())
    }
    /// The script lines that build C under `name`.
    pub fn script(&self, name: &str, ts: f64) -> String {
        let mut out = format!("{name} = {};\n", zpk_call(&[], &[], self.gain, ts));
        for element in &self.elements {
            out.push_str(&format!(
                "{name} = {name}*{};   % {}\n",
                element.factor(ts),
                element.describe()
            ));
        }
        out
    }
    /// C as a zpk formula, `20 (s + 1) / (s (s + 10))`.
    pub fn formula(&self, ts: f64) -> String {
        let v = if ts > 0. { "z" } else { "s" };
        let (zeros, poles) = self.roots(ts);
        let numerator = factors(&zeros, v);
        let denominator = factors(&poles, v);
        let mut text = short(self.gain);
        if !numerator.is_empty() {
            text = format!("{text} {}", numerator.join(" "));
        }
        match denominator.len() {
            0 => text,
            1 => format!("{text} / {}", denominator[0]),
            _ => format!("{text} / ({})", denominator.join(" ")),
        }
    }
    /// The movable root groups, in element order.
    pub fn handles(&self) -> Vec<Handle> {
        let mut out = Vec::new();
        for (i, element) in self.elements.iter().enumerate() {
            if !element.movable() {
                continue;
            }
            let (zeros, poles) = element.roots(0.);
            for (roots, zero) in [(zeros, true), (poles, false)] {
                if !roots.is_empty() {
                    out.push(Handle { element: i, zero });
                }
            }
        }
        out
    }
    /// Every root of C as drawn.
    pub fn markers(&self, ts: f64) -> Vec<Marker> {
        let mut out = Vec::new();
        for (i, element) in self.elements.iter().enumerate() {
            let (zeros, poles) = element.roots(ts);
            for (roots, zero) in [(zeros, true), (poles, false)] {
                for root in roots {
                    out.push(Marker {
                        handle: element.movable().then_some(Handle { element: i, zero }),
                        root,
                        zero,
                    });
                }
            }
        }
        out
    }
    /// Moves the handle's root to `q` in the loop's plane: a real root
    /// follows q's real part, a pair takes q's natural frequency and damping.
    pub fn move_root(&mut self, handle: Handle, q: [f64; 2], ts: f64) {
        let Some(element) = self.elements.get_mut(handle.element) else {
            return;
        };
        let (wn, zeta) = natural(Z::new(q[0], q[1].abs()), ts);
        let pair_ok = wn > 0. && wn.is_finite() && zeta.is_finite();
        let zeta = zeta.clamp(-MAX_ZETA, MAX_ZETA);
        if !q[0].is_finite() {
            return;
        }
        match element {
            Element::RealPole(x) | Element::RealZero(x) => *x = q[0],
            Element::Lead { zero, pole } | Element::Lag { zero, pole } => {
                *(if handle.zero { zero } else { pole }) = q[0];
            }
            Element::ComplexPoles { wn: w, zeta: z } | Element::ComplexZeros { wn: w, zeta: z }
                if pair_ok =>
            {
                (*w, *z) = (wn, zeta);
            }
            Element::Notch {
                wn: w,
                depth,
                width,
            } if pair_ok => {
                *w = wn;
                let damping = zeta.clamp(0., MAX_ZETA);
                if handle.zero {
                    *depth = damping;
                } else {
                    *width = damping.max(1e-3);
                }
            }
            _ => {}
        }
    }
    /// Corner frequency (rad/s) of the handle's root on the Bode plot.
    pub fn corner(&self, handle: Handle, ts: f64) -> Option<f64> {
        let real = |x: f64| {
            let w = if ts > 0. {
                if x > 0. { x.ln().abs() / ts } else { f64::NAN }
            } else {
                x.abs()
            };
            (w > 0. && w.is_finite()).then_some(w)
        };
        match *self.elements.get(handle.element)? {
            Element::RealPole(x) | Element::RealZero(x) => real(x),
            Element::Lead { zero, pole } | Element::Lag { zero, pole } => {
                real(if handle.zero { zero } else { pole })
            }
            Element::ComplexPoles { wn, .. }
            | Element::ComplexZeros { wn, .. }
            | Element::Notch { wn, .. } => Some(wn),
            Element::Integrator | Element::Differentiator => None,
        }
    }
    /// Moves the handle's corner frequency to `w` rad/s (side of the
    /// imaginary axis or unit circle kept).
    pub fn move_corner(&mut self, handle: Handle, w: f64, ts: f64) {
        if !(w > 0. && w.is_finite()) {
            return;
        }
        let w = if ts > 0. { w.min(0.999 * PI / ts) } else { w };
        let real = |x: f64| {
            if ts > 0. {
                (if x > 1. { w * ts } else { -w * ts }).exp()
            } else if x > 0. {
                w
            } else {
                -w
            }
        };
        let Some(element) = self.elements.get_mut(handle.element) else {
            return;
        };
        match element {
            Element::RealPole(x) | Element::RealZero(x) => *x = real(*x),
            Element::Lead { zero, pole } | Element::Lag { zero, pole } => {
                let x = if handle.zero { zero } else { pole };
                *x = real(*x);
            }
            Element::ComplexPoles { wn, .. }
            | Element::ComplexZeros { wn, .. }
            | Element::Notch { wn, .. } => *wn = w,
            Element::Integrator | Element::Differentiator => {}
        }
    }
    /// Real roots re-placed for a new sample time (same s-plane location);
    /// pairs keep their frequency and damping.
    pub fn convert(&mut self, from: f64, to: f64) {
        let map = |x: &mut f64| {
            let s = to_s(Z::new(*x, 0.), from);
            let moved = to_plane(Z::new(s.re, 0.), to).re;
            if s.im == 0. && moved.is_finite() {
                *x = moved;
            }
        };
        for element in &mut self.elements {
            match element {
                Element::RealPole(x) | Element::RealZero(x) => map(x),
                Element::Lead { zero, pole } | Element::Lag { zero, pole } => {
                    map(zero);
                    map(pole);
                }
                _ => {}
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Requirements

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Limit {
    pub on: bool,
    pub value: f64,
}
impl Limit {
    fn off(value: f64) -> Self {
        Self { on: false, value }
    }
    fn get(self) -> Option<f64> {
        (self.on && self.value.is_finite()).then_some(self.value)
    }
}

/// Optional design requirements: settling time <= s, overshoot <= %,
/// damping >= zeta, natural frequency >= rad/s (pole regions of the root
/// locus), gain margin >= dB and phase margin >= degrees (Bode).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Requirements {
    pub settling: Limit,
    pub overshoot: Limit,
    pub damping: Limit,
    pub frequency: Limit,
    pub gain_margin: Limit,
    pub phase_margin: Limit,
}
impl Default for Requirements {
    fn default() -> Self {
        Self {
            settling: Limit::off(2.),
            overshoot: Limit::off(10.),
            damping: Limit::off(0.5),
            frequency: Limit::off(1.),
            gain_margin: Limit::off(6.),
            phase_margin: Limit::off(45.),
        }
    }
}

/// Smallest damping ratio of a second-order step with this overshoot (%).
pub fn overshoot_damping(percent: f64) -> f64 {
    if percent <= 0. {
        return 1.;
    }
    let l = (percent / 100.).ln();
    -l / (PI * PI + l * l).sqrt()
}

/// One requirement check of the Output column.
#[derive(Clone, Debug, PartialEq)]
pub struct Check {
    pub label: String,
    pub pass: bool,
    pub measured: String,
}

impl Requirements {
    /// Any requirement that shades the root locus.
    pub fn pole_limits(&self) -> bool {
        [self.settling, self.overshoot, self.damping, self.frequency]
            .iter()
            .any(|l| l.get().is_some())
    }
    /// Whether a closed-loop pole at `p` (s-plane, or z-plane when ts > 0)
    /// lies in a region a pole requirement forbids.
    pub fn forbidden(&self, p: [f64; 2], ts: f64) -> bool {
        let s = to_s(Z::new(p[0], p[1]), ts);
        let (wn, zeta) = natural(Z::new(p[0], p[1]), ts);
        if let Some(t) = self.settling.get()
            && (s.re.is_nan() || s.re > -SETTLING_SIGMA / t)
        {
            return true;
        }
        let damping = [
            self.overshoot.get().map(overshoot_damping),
            self.damping.get(),
        ]
        .into_iter()
        .flatten()
        .fold(f64::NEG_INFINITY, f64::max);
        if damping > f64::NEG_INFINITY && (zeta.is_nan() || zeta < damping) {
            return true;
        }
        if let Some(w) = self.frequency.get()
            && (wn.is_nan() || wn < w)
        {
            return true;
        }
        false
    }
    /// Pass or fail of every requirement that is on, from the design's
    /// `stepinfo`, `damp` and `margin` values.
    pub fn checks(&self, d: &Design) -> Vec<Check> {
        let mut out = Vec::new();
        let min = |v: &[f64]| v.iter().copied().fold(f64::INFINITY, f64::min);
        if let Some(t) = self.settling.get() {
            out.push(Check {
                label: format!("Settling time ≤ {} s", short(t)),
                pass: d.settling <= t,
                measured: format!("{} s", short(d.settling)),
            });
        }
        if let Some(os) = self.overshoot.get() {
            out.push(Check {
                label: format!("Overshoot ≤ {} %", short(os)),
                pass: d.overshoot <= os,
                measured: format!("{} %", short(d.overshoot)),
            });
        }
        if let Some(z) = self.damping.get() {
            let least = min(&d.zeta);
            out.push(Check {
                label: format!("Damping ≥ {}", short(z)),
                pass: least >= z,
                measured: short(least),
            });
        }
        if let Some(w) = self.frequency.get() {
            let least = min(&d.wn);
            out.push(Check {
                label: format!("Natural frequency ≥ {} rad/s", short(w)),
                pass: least >= w,
                measured: format!("{} rad/s", short(least)),
            });
        }
        if let Some(g) = self.gain_margin.get() {
            let db = 20. * d.gain_margin.log10();
            out.push(Check {
                label: format!("Gain margin ≥ {} dB", short(g)),
                pass: db >= g,
                measured: format!("{} dB", short(db)),
            });
        }
        if let Some(m) = self.phase_margin.get() {
            out.push(Check {
                label: format!("Phase margin ≥ {}°", short(m)),
                pass: d.phase_margin >= m,
                measured: format!("{}°", short(d.phase_margin)),
            });
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Results

/// `rlocus` branches (one row of r each) and their gains.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Locus {
    pub branches: Vec<Vec<[f64; 2]>>,
    pub gains: Vec<f64>,
}

/// The gain of the locus point nearest to `point`, interpolated along the
/// nearest segment, and that point. Distances use `scale` (pixels per unit
/// on each axis) so "nearest" is nearest on screen.
pub fn locus_gain(locus: &Locus, point: [f64; 2], scale: [f64; 2]) -> Option<(f64, [f64; 2])> {
    let mut best: Option<(f64, f64, [f64; 2])> = None;
    for branch in &locus.branches {
        for (j, pair) in branch.windows(2).enumerate() {
            let (Some(&k0), Some(&k1)) = (locus.gains.get(j), locus.gains.get(j + 1)) else {
                continue;
            };
            let [a, b] = [pair[0], pair[1]];
            if !finite(&[a[0], a[1], b[0], b[1], k0, k1]) {
                continue;
            }
            let d = [(b[0] - a[0]) * scale[0], (b[1] - a[1]) * scale[1]];
            let q = [(point[0] - a[0]) * scale[0], (point[1] - a[1]) * scale[1]];
            let length = d[0] * d[0] + d[1] * d[1];
            let t = if length > 0. {
                ((q[0] * d[0] + q[1] * d[1]) / length).clamp(0., 1.)
            } else {
                0.
            };
            let distance = (q[0] - t * d[0]).hypot(q[1] - t * d[1]);
            if best.is_none_or(|(bd, ..)| distance < bd) {
                let at = [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])];
                best = Some((distance, k0 + t * (k1 - k0), at));
            }
        }
    }
    best.map(|(_, k, at)| (k, at))
}

#[derive(Clone, Debug)]
pub struct NicholsData {
    /// (phase deg, dB) of C*G*H.
    pub curve: Vec<[f64; 2]>,
    /// The `ngrid` contours.
    pub grid: Vec<Vec<[f64; 2]>>,
    /// The axes limits `ngrid` chose.
    pub x: [f64; 2],
    pub y: [f64; 2],
}

/// One computed design: what the private run returned for one C.
#[derive(Clone, Debug)]
pub struct Design {
    pub ts: f64,
    /// The C these results belong to (rlocus gains are relative to its gain).
    pub compensator: Compensator,
    pub locus: Arc<Locus>,
    /// Roots of 1 + C*G*H (rlocus at gain 1): the squares on the locus.
    pub closed: Vec<[f64; 2]>,
    /// Poles of the closed loop T.
    pub poles: Vec<[f64; 2]>,
    /// Poles and zeros of G*H.
    pub plant_poles: Vec<[f64; 2]>,
    pub plant_zeros: Vec<[f64; 2]>,
    /// (log10 w, dB) and (log10 w, degrees) of C*G*H.
    pub magnitude: Vec<[f64; 2]>,
    pub phase: Vec<[f64; 2]>,
    /// margin: gain margin (ratio), phase margin (deg), phase and gain crossovers.
    pub gain_margin: f64,
    pub phase_margin: f64,
    pub phase_crossover: f64,
    pub gain_crossover: f64,
    pub step: Vec<[f64; 2]>,
    /// r to u; None when its step has no finite answer.
    pub effort: Option<Vec<[f64; 2]>>,
    pub rise: f64,
    pub settling: f64,
    pub overshoot: f64,
    pub peak: f64,
    pub peak_time: f64,
    /// damp(T).
    pub wn: Vec<f64>,
    pub zeta: Vec<f64>,
    pub stable: bool,
    pub nichols: Option<NicholsData>,
}

const PROBE: [&str; 31] = [
    "app_ts",
    "app_rr",
    "app_ri",
    "app_k",
    "app_clr",
    "app_cli",
    "app_tpr",
    "app_tpi",
    "app_opr",
    "app_opi",
    "app_ozr",
    "app_ozi",
    "app_db",
    "app_ph",
    "app_w",
    "app_gm",
    "app_pm",
    "app_wcg",
    "app_wcp",
    "app_y",
    "app_t",
    "app_rise",
    "app_settle",
    "app_over",
    "app_peak",
    "app_tpeak",
    "app_wn",
    "app_zeta",
    "app_stable",
    "app_u",
    "app_tu",
];
const NICHOLS_PROBE: [&str; 6] = ["app_ndb", "app_np", "app_gx", "app_gy", "app_nx", "app_ny"];

fn matrix<'a>(outputs: &'a Outputs, name: &str) -> Result<&'a matrust::Matrix, String> {
    match outputs.get(name) {
        Some(Array::Numeric(m)) => Ok(m),
        _ => Err(format!(
            "Control System Designer: {name} is not a real array"
        )),
    }
}
fn values(outputs: &Outputs, name: &str) -> Result<Vec<f64>, String> {
    matrix(outputs, name).map(|m| m.data().to_vec())
}
fn scalar(outputs: &Outputs, name: &str) -> Result<f64, String> {
    values(outputs, name)?
        .first()
        .copied()
        .ok_or_else(|| format!("Control System Designer: {name} is empty"))
}
fn points(outputs: &Outputs, re: &str, im: &str) -> Result<Vec<[f64; 2]>, String> {
    Ok(values(outputs, re)?
        .into_iter()
        .zip(values(outputs, im)?)
        .map(|(x, y)| [x, y])
        .collect())
}
/// Finite (x, y) points; `log_x` plots x on a log10 axis.
fn curve(x: &[f64], y: &[f64], log_x: bool) -> Vec<[f64; 2]> {
    x.iter()
        .zip(y)
        .map(|(x, y)| [if log_x { x.log10() } else { *x }, *y])
        .filter(|p| p[0].is_finite() && p[1].is_finite())
        .collect()
}
/// Runs of finite points.
fn segments(points: impl IntoIterator<Item = [f64; 2]>) -> Vec<Vec<[f64; 2]>> {
    let mut out = vec![Vec::new()];
    for p in points {
        if p[0].is_finite() && p[1].is_finite() {
            out.last_mut().unwrap().push(p);
        } else if !out.last().unwrap().is_empty() {
            out.push(Vec::new());
        }
    }
    out.retain(|s| s.len() > 1);
    out
}
/// y of a polyline at x (x increasing), linear between points.
fn interpolate(curve: &[[f64; 2]], x: f64) -> Option<f64> {
    let i = curve.partition_point(|p| p[0] < x);
    match (i.checked_sub(1).and_then(|j| curve.get(j)), curve.get(i)) {
        (Some(a), Some(b)) if b[0] > a[0] => {
            Some(a[1] + (x - a[0]) / (b[0] - a[0]) * (b[1] - a[1]))
        }
        (_, Some(b)) if b[0] == x => Some(b[1]),
        _ => None,
    }
}

impl Design {
    pub fn from_outputs(
        outputs: &Outputs,
        compensator: Compensator,
        nichols: bool,
    ) -> Result<Self, String> {
        let r = matrix(outputs, "app_rr")?;
        let ri = values(outputs, "app_ri")?;
        let (rows, cols) = (r.rows(), r.cols());
        let rr = r.data();
        if ri.len() != rr.len() {
            return Err("Control System Designer: rlocus returned mismatched parts".into());
        }
        let branches = (0..rows)
            .map(|b| {
                (0..cols)
                    .map(|j| [rr[b + j * rows], ri[b + j * rows]])
                    .collect()
            })
            .collect();
        let w = values(outputs, "app_w")?;
        let effort_y = values(outputs, "app_u")?;
        let nichols = if nichols {
            let limits = |name| -> Result<[f64; 2], String> {
                match values(outputs, name)?.as_slice() {
                    [a, b] => Ok([*a, *b]),
                    _ => Err(format!("Control System Designer: {name} is not two limits")),
                }
            };
            let gx = values(outputs, "app_gx")?;
            let gy = values(outputs, "app_gy")?;
            Some(NicholsData {
                curve: curve(
                    &values(outputs, "app_np")?,
                    &values(outputs, "app_ndb")?,
                    false,
                ),
                grid: segments(gx.into_iter().zip(gy).map(|(x, y)| [x, y])),
                x: limits("app_nx")?,
                y: limits("app_ny")?,
            })
        } else {
            None
        };
        Ok(Self {
            ts: scalar(outputs, "app_ts")?,
            compensator,
            locus: Arc::new(Locus {
                branches,
                gains: values(outputs, "app_k")?,
            }),
            closed: points(outputs, "app_clr", "app_cli")?,
            poles: points(outputs, "app_tpr", "app_tpi")?,
            plant_poles: points(outputs, "app_opr", "app_opi")?,
            plant_zeros: points(outputs, "app_ozr", "app_ozi")?,
            magnitude: curve(&w, &values(outputs, "app_db")?, true),
            phase: curve(&w, &values(outputs, "app_ph")?, true),
            gain_margin: scalar(outputs, "app_gm")?,
            phase_margin: scalar(outputs, "app_pm")?,
            phase_crossover: scalar(outputs, "app_wcg")?,
            gain_crossover: scalar(outputs, "app_wcp")?,
            step: curve(
                &values(outputs, "app_t")?,
                &values(outputs, "app_y")?,
                false,
            ),
            effort: (!effort_y.is_empty())
                .then(|| values(outputs, "app_tu").map(|t| curve(&t, &effort_y, false)))
                .transpose()?,
            rise: scalar(outputs, "app_rise")?,
            settling: scalar(outputs, "app_settle")?,
            overshoot: scalar(outputs, "app_over")?,
            peak: scalar(outputs, "app_peak")?,
            peak_time: scalar(outputs, "app_tpeak")?,
            wn: values(outputs, "app_wn")?,
            zeta: values(outputs, "app_zeta")?,
            stable: scalar(outputs, "app_stable")? != 0.,
            nichols,
        })
    }
    /// The gain margin as `x dB at w rad/s`.
    pub fn gain_margin_text(&self) -> String {
        if self.gain_margin.is_finite() && self.gain_margin > 0. {
            format!(
                "{} dB at {} rad/s",
                short(20. * self.gain_margin.log10()),
                short(self.phase_crossover)
            )
        } else {
            short(self.gain_margin)
        }
    }
    pub fn phase_margin_text(&self) -> String {
        if self.phase_margin.is_finite() {
            format!(
                "{}° at {} rad/s",
                short(self.phase_margin),
                short(self.gain_crossover)
            )
        } else {
            short(self.phase_margin)
        }
    }
}

/// Poles as text: real ones, and `a ± bi` once per conjugate pair.
pub fn pole_texts(poles: &[[f64; 2]]) -> Vec<String> {
    // Rounding can leave a real pole with a tiny imaginary part.
    let real = |p: &[f64; 2]| p[1].abs() <= 1e-9 * p[0].abs().max(1.);
    let mut sorted: Vec<[f64; 2]> = poles
        .iter()
        .copied()
        .filter(|p| p[1] > 0. || real(p))
        .collect();
    sorted.sort_by(|a, b| b[0].total_cmp(&a[0]).then(a[1].total_cmp(&b[1])));
    sorted
        .into_iter()
        .map(|p| {
            let [re, im] = p;
            if real(&p) {
                short(re)
            } else {
                format!("{} ± {}i", short(re), short(im))
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// The app

/// Which plots are on: they are drawn and their commands go in the script.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Views {
    pub root_locus: bool,
    pub bode: bool,
    pub nichols: bool,
    pub step: bool,
    /// The r to u step.
    pub effort: bool,
}
impl Default for Views {
    fn default() -> Self {
        Self {
            root_locus: true,
            bode: true,
            nichols: false,
            step: true,
            effort: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Editor {
    RootLocus,
    Bode,
    Nichols,
}

/// A drag in progress; the plot keeps the bounds it had when it started.
#[derive(Clone, Debug)]
enum Drag {
    /// A closed-loop pole along the locus the design had at the start.
    Gain {
        locus: Arc<Locus>,
        gain: f64,
        bounds: egui_plot::PlotBounds,
    },
    /// A root of C on the root locus.
    Root {
        handle: Handle,
        bounds: egui_plot::PlotBounds,
    },
    /// The Bode magnitude or the Nichols curve up or down (start dB, gain).
    Level {
        editor: Editor,
        start: f64,
        gain: f64,
        bounds: egui_plot::PlotBounds,
    },
    /// A corner frequency along the Bode magnitude.
    Corner {
        handle: Handle,
        bounds: egui_plot::PlotBounds,
    },
}
impl Drag {
    fn bounds(&self, editor: Editor) -> Option<egui_plot::PlotBounds> {
        match self {
            Self::Gain { bounds, .. } | Self::Root { bounds, .. } => {
                (editor == Editor::RootLocus).then_some(*bounds)
            }
            Self::Level {
                editor: e, bounds, ..
            } => (*e == editor).then_some(*bounds),
            Self::Corner { bounds, .. } => (editor == Editor::Bode).then_some(*bounds),
        }
    }
}

pub struct ControlSystemDesigner {
    pub g: Source,
    pub h: Source,
    pub f: Source,
    /// Sample time of typed models (s); 0 or empty is continuous time.
    pub sample_time: String,
    pub architecture: Architecture,
    pub compensator: Compensator,
    pub requirements: Requirements,
    pub views: Views,
    pub c_name: String,
    pub c_edited: bool,
    pub t_name: String,
    pub t_edited: bool,
    /// Last good design; errors keep it on screen.
    pub current: Option<Arc<Design>>,
    undo: Vec<Compensator>,
    redo: Vec<Compensator>,
    /// The compensator the undo history last recorded.
    committed: Compensator,
    /// A compensator widget is being dragged or typed in.
    editing: bool,
    /// Sample time of the last valid setup.
    loop_ts: Option<f64>,
    drag: Option<Drag>,
    /// Where the root locus was right-clicked.
    menu_at: [f64; 2],
    /// Pixel size of the root locus last frame (for equal axis scales).
    locus_size: egui::Vec2,
    regions: Option<(u64, egui::TextureHandle)>,
}
impl Default for ControlSystemDesigner {
    fn default() -> Self {
        Self {
            g: Source::typed("1", "1 6 5 0"),
            h: Source::typed("1", "1"),
            f: Source::typed("1", "1"),
            sample_time: "0".into(),
            architecture: Architecture::Forward,
            compensator: Compensator::default(),
            requirements: Requirements::default(),
            views: Views::default(),
            c_name: String::new(),
            c_edited: false,
            t_name: String::new(),
            t_edited: false,
            current: None,
            undo: Vec::new(),
            redo: Vec::new(),
            committed: Compensator::default(),
            editing: false,
            loop_ts: None,
            drag: None,
            menu_at: [0.; 2],
            locus_size: egui::vec2(400., 300.),
            regions: None,
        }
    }
}

impl ControlSystemDesigner {
    pub fn source(&self, role: Role) -> &Source {
        match role {
            Role::Plant => &self.g,
            Role::Sensor => &self.h,
            Role::Prefilter => &self.f,
        }
    }
    fn source_mut(&mut self, role: Role) -> &mut Source {
        match role {
            Role::Plant => &mut self.g,
            Role::Sensor => &mut self.h,
            Role::Prefilter => &mut self.f,
        }
    }
    /// The typed models' sample time.
    pub fn typed_ts(&self) -> Result<f64, String> {
        let text = self.sample_time.trim();
        if text.is_empty() {
            return Ok(0.);
        }
        text.parse::<f64>()
            .ok()
            .filter(|t| t.is_finite() && *t >= 0.)
            .ok_or_else(|| {
                "Sample time: type 0 for continuous time or a positive number of seconds".into()
            })
    }
    /// The loop of the script: model expressions, definitions and names.
    pub fn setup(&self, ws: &Snapshot) -> Result<Setup, String> {
        let mut taken: Vec<String> = Vec::new();
        for role in Role::ALL {
            let source = self.source(role);
            if !source.use_variable {
                continue;
            }
            if source.variable.is_empty() {
                return Err(format!(
                    "{}: choose a workspace variable holding a tf, zpk or ss model",
                    role.label()
                ));
            }
            if !ws.contains(&source.variable) {
                return Err(format!(
                    "{}: workspace variable {} does not exist",
                    role.label(),
                    source.variable
                ));
            }
            taken.push(source.variable.clone());
        }
        let ts = if self.g.use_variable {
            ws.get(&self.g.variable).and_then(model_ts).ok_or_else(|| {
                format!("Plant G: {} is not a tf, zpk or ss model", self.g.variable)
            })?
        } else {
            self.typed_ts()?
        };
        let ts_argument = if ts > 0. {
            format!(", {}", number(ts))
        } else {
            String::new()
        };
        let mut definitions = Vec::new();
        let mut defined = Vec::new();
        let mut expressions = Vec::new();
        for role in Role::ALL {
            let source = self.source(role);
            if source.use_variable {
                expressions.push(source.variable.clone());
                continue;
            }
            let numerator =
                parse_coefficients(&source.numerator, &format!("{} numerator", role.label()))?;
            let denominator = parse_coefficients(
                &source.denominator,
                &format!("{} denominator", role.label()),
            )?;
            if role != Role::Plant
                && numerator.len() == 1
                && denominator.len() == 1
                && numerator[0] == denominator[0]
            {
                expressions.push("1".into());
                continue;
            }
            let name = unused_name(role.base(), |n| {
                ws.contains(n) || taken.iter().any(|t| t == n)
            });
            taken.push(name.clone());
            definitions.push(format!(
                "{name} = tf({}, {}{ts_argument});",
                vector(&numerator),
                vector(&denominator)
            ));
            defined.push(name.clone());
            expressions.push(name);
        }
        let output = |edited: bool, typed: &str, base: &str, taken: &[String]| {
            if edited {
                typed.trim().to_owned()
            } else {
                unused_name(base, |n| ws.contains(n) || taken.iter().any(|t| t == n))
            }
        };
        let c = output(self.c_edited, &self.c_name, "C", &taken);
        super::check_export_name(&c)?;
        if !matrust::files::valid_name(&c) {
            return Err("The compensator name is not a valid variable name".into());
        }
        if taken.contains(&c) {
            return Err(format!(
                "The compensator name {c} is a model of the loop: choose another"
            ));
        }
        taken.push(c.clone());
        let t = output(self.t_edited, &self.t_name, "T", &taken);
        super::check_export_name(&t)?;
        if !matrust::files::valid_name(&t) {
            return Err("The closed loop name is not a valid variable name".into());
        }
        if taken.contains(&t) {
            return Err(format!(
                "The closed loop name {t} is already used by the loop: choose another"
            ));
        }
        let [g, h, f]: [String; 3] = expressions.try_into().expect("three models");
        Ok(Setup {
            g,
            h,
            f,
            definitions,
            defined,
            c,
            t,
            ts,
        })
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty() || self.compensator != self.committed
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty() && self.compensator == self.committed
    }
    /// Records a finished compensator edit in the undo history.
    pub fn commit(&mut self) {
        if self.compensator != self.committed {
            let before = std::mem::replace(&mut self.committed, self.compensator.clone());
            self.undo.push(before);
            if self.undo.len() > MAX_HISTORY {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
    }
    pub fn undo(&mut self) -> bool {
        self.commit();
        let Some(previous) = self.undo.pop() else {
            return false;
        };
        self.redo
            .push(std::mem::replace(&mut self.compensator, previous.clone()));
        self.committed = previous;
        true
    }
    pub fn redo(&mut self) -> bool {
        self.commit();
        let Some(next) = self.redo.pop() else {
            return false;
        };
        self.undo
            .push(std::mem::replace(&mut self.compensator, next.clone()));
        self.committed = next;
        true
    }
    /// The loop's sample time as of the last valid setup.
    pub fn ts(&self) -> f64 {
        self.loop_ts.unwrap_or(0.)
    }
    /// Adds an element; right-click places real roots at q's real part and
    /// pairs at q's frequency and damping.
    pub fn add_element(&mut self, kind: ElementKind, at: Option<[f64; 2]>) {
        if self.compensator.elements.len() >= MAX_ELEMENTS {
            return;
        }
        let ts = self.ts();
        let mut element = Element::new(kind, ts);
        if let Some(q) = at {
            let i = self.compensator.elements.len();
            self.compensator.elements.push(element);
            let zero = matches!(kind, ElementKind::RealZero | ElementKind::ComplexZeros);
            self.compensator
                .move_root(Handle { element: i, zero }, q, ts);
            element = self.compensator.elements.pop().unwrap();
        }
        self.compensator.elements.push(element);
    }
}

/// C's gain when a closed-loop pole is dragged to `point`: the gain the
/// locus was computed with times rlocus's gain at the nearest locus point.
pub fn dragged_gain(locus: &Locus, gain: f64, point: [f64; 2], scale: [f64; 2]) -> Option<f64> {
    let (k, _) = locus_gain(locus, point, scale)?;
    Some(gain * k.max(MIN_FACTOR))
}

/// `[1 2 3]`.
fn vector(values: &[f64]) -> String {
    let items: Vec<String> = values.iter().map(|v| number(*v)).collect();
    format!("[{}]", items.join(" "))
}

impl App for ControlSystemDesigner {
    fn kind(&self) -> AppKind {
        AppKind::ControlSystemDesigner
    }
    fn script(&self, ws: &Snapshot) -> Result<String, String> {
        let setup = self.setup(ws)?;
        self.compensator.check(setup.ts)?;
        let mut script = format!(
            "% Loop: {}, {}.\n",
            self.architecture.label(),
            self.architecture.formula().replace('·', "*")
        );
        let mut defined = setup.definitions.iter();
        for (role, expression) in Role::ALL.into_iter().zip([&setup.g, &setup.h, &setup.f]) {
            if self.source(role).use_variable {
                script.push_str(&format!(
                    "% {} is the workspace variable {expression}.\n",
                    role.base()
                ));
            } else if expression != "1"
                && let Some(line) = defined.next()
            {
                script.push_str(line);
                script.push('\n');
            }
        }
        script.push_str(&self.compensator.script(&setup.c, setup.ts));
        script.push_str(&format!(
            "{} = {};\n",
            setup.t,
            setup.closed_loop(self.architecture)
        ));
        let open = setup.open_loop();
        if self.views.root_locus {
            script.push_str(&format!("figure; rlocus({open})\n"));
        }
        if self.views.bode {
            script.push_str(&format!("figure; margin({open})\n"));
        }
        if self.views.nichols {
            script.push_str(&format!("figure; nichols({open}); ngrid\n"));
        }
        if self.views.step {
            script.push_str(&format!("figure; step({})\n", setup.t));
        }
        if self.views.effort {
            script.push_str(&format!(
                "figure; step({})\n",
                setup.effort(self.architecture)
            ));
        }
        Ok(script)
    }
    fn inputs(&self) -> Vec<String> {
        Role::ALL
            .into_iter()
            .map(|role| self.source(role))
            .filter(|s| s.use_variable && !s.variable.is_empty())
            .map(|s| s.variable.clone())
            .collect()
    }
    fn probe(&self, ws: &Snapshot) -> (String, Vec<String>) {
        let Ok(setup) = self.setup(ws) else {
            return (String::new(), Vec::new());
        };
        let private = setup.private();
        // Private copies first, so no probe name can shadow the user's names.
        let mut code = format!(
            "[app_C, app_G, app_H, app_F, app_T] = deal({}, {}, {}, {}, {});\n\
             app_L = {};\napp_GH = app_G*app_H;\napp_ts = app_L.Ts;\n\
             [app_r, app_k] = rlocus(app_L);\n\
             app_rr = real(app_r); app_ri = imag(app_r);\n\
             app_cl = rlocus(app_L, 1); app_clr = real(app_cl(:)); app_cli = imag(app_cl(:));\n\
             app_tp = pole(app_T); app_tpr = real(app_tp(:)); app_tpi = imag(app_tp(:));\n\
             app_op = pole(app_GH); app_opr = real(app_op(:)); app_opi = imag(app_op(:));\n\
             app_oz = zero(app_GH); app_ozr = real(app_oz(:)); app_ozi = imag(app_oz(:));\n\
             [app_mag, app_ph, app_w] = bode(app_L);\n\
             app_db = 20*log10(app_mag(:)); app_ph = app_ph(:); app_w = app_w(:);\n\
             [app_gm, app_pm, app_wcg, app_wcp] = margin(app_L);\n\
             [app_y, app_t] = step(app_T);\n\
             app_S = stepinfo(app_T);\n\
             app_rise = app_S.RiseTime; app_settle = app_S.SettlingTime;\n\
             app_over = app_S.Overshoot; app_peak = app_S.Peak; app_tpeak = app_S.PeakTime;\n\
             [app_wn, app_zeta] = damp(app_T);\n\
             app_stable = double(isstable(app_T));\n\
             try\napp_U = {};\n[app_u, app_tu] = step(app_U);\ncatch\napp_u = []; app_tu = [];\nend\n",
            setup.c,
            setup.g,
            setup.h,
            setup.f,
            setup.t,
            private.open_loop(),
            private.effort(self.architecture),
        );
        let mut names: Vec<String> = PROBE.iter().map(|s| (*s).to_owned()).collect();
        if self.views.nichols {
            code.push_str(
                "[app_nm, app_np] = nichols(app_L); app_ndb = 20*log10(app_nm(:)); app_np = app_np(:);\n\
                 figure; nichols(app_L); ngrid;\n\
                 app_kids = get(gca, 'Children'); app_gx = []; app_gy = [];\n\
                 for app_i = 1:numel(app_kids)\n\
                 if strcmp(get(app_kids(app_i), 'Type'), 'line') && strcmp(get(app_kids(app_i), 'LineStyle'), ':')\n\
                 app_gx = [app_gx, NaN, get(app_kids(app_i), 'XData')];\n\
                 app_gy = [app_gy, NaN, get(app_kids(app_i), 'YData')];\n\
                 end\nend\n\
                 app_nx = get(gca, 'XLim'); app_ny = get(gca, 'YLim');\n",
            );
            names.extend(NICHOLS_PROBE.iter().map(|s| (*s).to_owned()));
        }
        (code, names)
    }
    fn prepare(&mut self, ws: &Snapshot) {
        if !self.editing && self.drag.is_none() {
            self.commit();
        }
        self.editing = false;
        if let Ok(setup) = self.setup(ws) {
            if let Some(old) = self.loop_ts
                && old != setup.ts
            {
                self.compensator.convert(old, setup.ts);
                self.commit();
            }
            self.loop_ts = Some(setup.ts);
        }
    }
    fn receive(&mut self, outputs: &Outputs) -> Result<(), String> {
        let design = Design::from_outputs(outputs, self.compensator.clone(), self.views.nichols)?;
        self.current = Some(Arc::new(design));
        Ok(())
    }
    fn inputs_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        field(ui, "Loop", |ui| {
            for architecture in Architecture::ALL {
                if crate::tokens::toggle(
                    ui,
                    self.architecture == architecture,
                    architecture.label(),
                )
                .clicked()
                {
                    self.architecture = architecture;
                }
            }
            crate::tokens::helper(ui, self.architecture.formula());
        });
        self.source_ui(ui, ws, Role::Plant);
        if self.g.use_variable {
            crate::tokens::helper(
                ui,
                format!(
                    "Sample time {} from G; typed H and F use it too.",
                    match self.loop_ts {
                        Some(t) if t > 0. => format!("{} s", short(t)),
                        Some(_) => "0 (continuous)".into(),
                        None => "unknown".into(),
                    }
                ),
            );
        } else {
            ui.horizontal(|ui| {
                ui.label("Sample time (s)");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add(
                        crate::tokens::input(egui::TextEdit::singleline(&mut self.sample_time))
                            .desired_width(96.),
                    );
                });
            });
            crate::tokens::helper(ui, "0 for continuous time; typed H and F use it too");
        }
        ui.add_space(crate::tokens::S);
        let unity = |s: &Source| {
            !s.use_variable && s.numerator.trim() == "1" && s.denominator.trim() == "1"
        };
        let title = match (unity(&self.h), unity(&self.f)) {
            (true, true) => "Sensor and prefilter (H = 1, F = 1)",
            (true, false) => "Sensor and prefilter (H = 1)",
            (false, true) => "Sensor and prefilter (F = 1)",
            (false, false) => "Sensor and prefilter",
        };
        egui::CollapsingHeader::new(title)
            .id_salt("control_system_designer_sensor_prefilter")
            .default_open(!(unity(&self.h) && unity(&self.f)))
            .show(ui, |ui| {
                crate::tokens::helper(ui, "Typed 1 | 1 leaves H or F out of the loop");
                ui.add_space(crate::tokens::XS);
                self.source_ui(ui, ws, Role::Sensor);
                self.source_ui(ui, ws, Role::Prefilter);
            });
        ui.add_space(crate::tokens::XS);
        section(ui);
        self.compensator_ui(ui);
        section(ui);
        self.requirements_ui(ui);
    }
    fn main_caption(&self) -> &'static str {
        "DESIGN"
    }
    fn main_ui(&mut self, ui: &mut egui::Ui) {
        // A drag ends with the button; a plot that went away mid-drag
        // must not keep its bounds frozen.
        if self.drag.is_some() && !ui.input(|i| i.pointer.any_down()) {
            self.drag = None;
        }
        self.views_ui(ui);
        ui.add_space(crate::tokens::XS);
        let Some(design) = self.current.clone() else {
            crate::tokens::helper(ui, "No design yet: the status line says why.");
            return;
        };
        let editors: Vec<Editor> = [
            (self.views.root_locus, Editor::RootLocus),
            (self.views.bode, Editor::Bode),
            (self.views.nichols, Editor::Nichols),
        ]
        .into_iter()
        .filter_map(|(on, editor)| on.then_some(editor))
        .collect();
        let analysis = self.views.step || self.views.effort;
        if !editors.is_empty() {
            caption_line(
                ui,
                "Drag a square (closed-loop pole) or the Bode or Nichols curve to set the gain; \
                 drag C's poles (x) and zeros (o) to move them; right-click the root locus to add one.",
            );
        }
        // Share the available height between editors and step responses.
        let width = ui.available_width();
        let height = (ui.available_height() - crate::tokens::S).max(96.);
        let top = match (editors.is_empty(), analysis) {
            (true, false) => {
                crate::tokens::helper(ui, "Turn on an editor or the step plot above.");
                return;
            }
            (true, true) => 0.,
            (false, true) => (height * 0.6).floor(),
            (false, false) => height,
        };
        // A rotated axis title must fit the data frame after the horizontal
        // tick labels and title take their share. The compact host can scroll.
        let axis_height = |title: &str| {
            let font = egui::TextStyle::Body.resolve(ui.style());
            ui.painter()
                .layout_no_wrap(title.into(), font, ui.visuals().text_color())
                .size()
                .x
                + 2. * crate::tokens::ROW
        };
        let compact = ui.stack().contained_in(egui::UiKind::ScrollArea);
        let both_responses = self.views.step && self.views.effort;
        let min_response_height = if analysis && (compact || both_responses) {
            axis_height("Output y").max(axis_height("Input u")) + crate::tokens::ROW
        } else {
            0.
        };
        let top = if compact && self.views.bode {
            let half = axis_height("Magnitude (dB)").max(axis_height("Phase (deg)"));
            top.max(crate::tokens::ROW + crate::tokens::S + 2. * half)
        } else {
            top
        };
        if !editors.is_empty() {
            let n = editors.len() as f32;
            let each = ((width - crate::tokens::S * (n - 1.)) / n)
                .floor()
                .max(120.);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = crate::tokens::S;
                for editor in editors {
                    ui.allocate_ui_with_layout(
                        egui::vec2(each, top),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_width(each);
                            let size = egui::vec2(each, top - crate::tokens::ROW);
                            match editor {
                                Editor::RootLocus => self.locus_ui(ui, &design, size),
                                Editor::Bode => self.bode_ui(ui, &design, size),
                                Editor::Nichols => self.nichols_ui(ui, &design, size),
                            }
                        },
                    );
                }
            });
        }
        if analysis {
            if top > 0. {
                ui.add_space(crate::tokens::S);
            }
            let available = ui.available_height() - crate::tokens::XS;
            let stacked_min = 2. * min_response_height + crate::tokens::S;
            if both_responses && !compact && available < stacked_min {
                // A short, wide panel has room for both time axes on one row.
                let height = (available - crate::tokens::ROW).max(40.);
                ui.columns(2, |columns| {
                    step_response_ui(&mut columns[0], &design, height);
                    control_effort_ui(&mut columns[1], &design, height);
                });
            } else {
                let row = if both_responses {
                    (available - crate::tokens::S) / 2.
                } else {
                    available
                };
                let row = if compact {
                    row.max(min_response_height)
                } else {
                    row
                };
                let height = (row - crate::tokens::ROW).max(40.);
                if self.views.step {
                    step_response_ui(ui, &design, height);
                }
                if both_responses {
                    ui.add_space(crate::tokens::S);
                }
                if self.views.effort {
                    control_effort_ui(ui, &design, height);
                }
            }
        }
    }
    fn results(&self) -> Vec<(&'static str, String)> {
        let Some(d) = &self.current else {
            return ["Gain", "Closed-loop poles", "Gain margin", "Phase margin"]
                .into_iter()
                .map(|label| (label, "—".to_owned()))
                .collect();
        };
        let mut rows = vec![("Gain", short(d.compensator.gain))];
        let poles = pole_texts(&d.poles);
        const SHOWN: usize = 8;
        if poles.is_empty() {
            rows.push(("Closed-loop poles", "none".into()));
        }
        for (i, text) in poles.iter().take(SHOWN).enumerate() {
            rows.push((if i == 0 { "Closed-loop poles" } else { "" }, text.clone()));
        }
        if poles.len() > SHOWN {
            rows.push(("", format!("and {} more", poles.len() - SHOWN)));
        }
        rows.extend([
            ("Gain margin", d.gain_margin_text()),
            ("Phase margin", d.phase_margin_text()),
            ("Rise time", format!("{} s", short(d.rise))),
            ("Settling time", format!("{} s", short(d.settling))),
            ("Overshoot", format!("{} %", short(d.overshoot))),
            ("Peak", short(d.peak)),
            (
                "Closed loop stable",
                if d.stable { "yes" } else { "no" }.to_owned(),
            ),
        ]);
        rows
    }
    fn names_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        let p = crate::tokens::palette(ui.ctx());
        ui.label(egui::RichText::new("C (zpk)").color(p.text_muted));
        ui.add(
            egui::Label::new(
                egui::RichText::new(self.compensator.formula(self.ts())).color(p.text),
            )
            .wrap(),
        );
        if let Some(d) = &self.current {
            let checks = self.requirements.checks(d);
            if !checks.is_empty() {
                ui.add_space(crate::tokens::S);
                ui.label(egui::RichText::new("Requirements").color(p.text_muted));
                for check in checks {
                    let (word, color) = if check.pass {
                        ("pass", p.success)
                    } else {
                        ("fail", p.error)
                    };
                    ui.horizontal_wrapped(|ui| {
                        ui.label(egui::RichText::new(word).color(color));
                        ui.label(
                            egui::RichText::new(format!("{} ({})", check.label, check.measured))
                                .color(p.text),
                        );
                    });
                }
            }
        }
        ui.add_space(crate::tokens::M);
        let setup = self.setup(ws).ok();
        if !self.c_edited {
            self.c_name = setup.as_ref().map_or_else(|| "C".into(), |s| s.c.clone());
        }
        let before = self.c_name.clone();
        name_field(ui, "Compensator name", &mut self.c_name, ws);
        self.c_edited |= self.c_name != before;
        if !self.t_edited {
            self.t_name = setup.as_ref().map_or_else(|| "T".into(), |s| s.t.clone());
        }
        let before = self.t_name.clone();
        name_field(ui, "Closed loop name", &mut self.t_name, ws);
        self.t_edited |= self.t_name != before;
    }
    fn exported(&self, ws: &Snapshot) -> Vec<String> {
        match self.setup(ws) {
            Ok(setup) => [setup.c, setup.t]
                .into_iter()
                .chain(setup.defined)
                .collect(),
            Err(_) => Vec::new(),
        }
    }
    fn summary(&self) -> Option<String> {
        let d = self.current.as_ref()?;
        let failed = self
            .requirements
            .checks(d)
            .iter()
            .filter(|c| !c.pass)
            .count();
        let mut text = format!(
            "Gain {} · phase margin {} · gain margin {} · closed loop {}",
            short(d.compensator.gain),
            if d.phase_margin.is_finite() {
                format!("{}°", short(d.phase_margin))
            } else {
                short(d.phase_margin)
            },
            if d.gain_margin.is_finite() && d.gain_margin > 0. {
                format!("{} dB", short(20. * d.gain_margin.log10()))
            } else {
                short(d.gain_margin)
            },
            if d.stable { "stable" } else { "unstable" }
        );
        if failed > 0 {
            text.push_str(&format!(
                " · {failed} requirement{} not met",
                if failed == 1 { "" } else { "s" }
            ));
        }
        Some(text)
    }
    fn export_label(&self) -> &'static str {
        "Export C and T"
    }
}

// ---------------------------------------------------------------------------
// Inputs

fn section(ui: &mut egui::Ui) {
    ui.add_space(crate::tokens::XS);
    ui.separator();
    ui.add_space(crate::tokens::XS);
}

/// One numeric parameter: label left, value right.
fn parameter(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f64,
    range: std::ops::RangeInclusive<f64>,
    editing: &mut bool,
) {
    let p = crate::tokens::palette(ui.ctx());
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(label).color(p.text_muted));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let speed = (value.abs() * 0.005).max(1e-4);
            let response = ui.add(
                egui::DragValue::new(value)
                    .speed(speed)
                    .range(range)
                    .max_decimals(6),
            );
            *editing |= response.dragged() || response.has_focus();
        });
    });
}

impl ControlSystemDesigner {
    fn source_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot, role: Role) {
        let models: Vec<(String, &str)> = ws
            .lti_models()
            .map(|(name, class)| (name.to_owned(), class))
            .collect();
        let source = self.source_mut(role);
        // Label left, source right: one row per model.
        ui.horizontal(|ui| {
            ui.label(role.label());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if crate::tokens::toggle(ui, source.use_variable, "Workspace").clicked() {
                    source.use_variable = true;
                }
                if crate::tokens::toggle(ui, !source.use_variable, "Typed").clicked() {
                    source.use_variable = false;
                }
            });
        });
        ui.add_space(crate::tokens::XS);
        if source.use_variable {
            if source.variable.is_empty()
                && let Some((name, _)) = models.first()
            {
                source.variable = name.clone();
            }
            let shown = if source.variable.is_empty() {
                "Choose a model".to_owned()
            } else {
                source.variable.clone()
            };
            egui::ComboBox::from_id_salt(("csd_model", role.base()))
                .width(ui.available_width())
                .selected_text(shown)
                .show_ui(ui, |ui| {
                    for (name, class) in &models {
                        ui.selectable_value(
                            &mut source.variable,
                            name.clone(),
                            format!("{name} ({class})"),
                        );
                    }
                });
            if models.is_empty() {
                crate::tokens::helper(ui, "No tf, zpk or ss variables in the workspace");
            }
            ui.add_space(crate::tokens::S);
        } else {
            let width = ui.available_width();
            ui.horizontal(|ui| {
                let half = ((width - crate::tokens::S) / 2.).max(60.);
                ui.add(
                    crate::tokens::input(
                        egui::TextEdit::singleline(&mut source.numerator).hint_text("Numerator"),
                    )
                    .desired_width(half),
                )
                .on_hover_text("Numerator coefficients, descending powers");
                ui.add(
                    crate::tokens::input(
                        egui::TextEdit::singleline(&mut source.denominator)
                            .hint_text("Denominator"),
                    )
                    .desired_width(half),
                )
                .on_hover_text("Denominator coefficients, descending powers");
            });
            if role == Role::Plant {
                crate::tokens::helper(ui, "Numerator | denominator, descending powers");
            }
            ui.add_space(crate::tokens::S);
        }
    }

    fn compensator_ui(&mut self, ui: &mut egui::Ui) {
        let p = crate::tokens::palette(ui.ctx());
        let ts = self.ts();
        ui.label("Compensator C");
        ui.add(
            egui::Label::new(
                egui::RichText::new(format!("C = {}", self.compensator.formula(ts))).color(p.text),
            )
            .wrap(),
        );
        ui.add_space(crate::tokens::XS);
        let mut editing = false;
        parameter(
            ui,
            "Gain",
            &mut self.compensator.gain,
            -1e12..=1e12,
            &mut editing,
        );
        let mut remove = None;
        let real_range = -1e9..=1e9;
        let unit = if ts > 0. { "z" } else { "s" };
        for (i, element) in self.compensator.elements.iter_mut().enumerate() {
            ui.add_space(crate::tokens::XS);
            ui.horizontal(|ui| {
                ui.label(element.kind().label());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("Remove").clicked() {
                        remove = Some(i);
                    }
                });
            });
            match element {
                Element::RealPole(x) | Element::RealZero(x) => {
                    parameter(
                        ui,
                        &format!("At {unit} ="),
                        x,
                        real_range.clone(),
                        &mut editing,
                    );
                }
                Element::ComplexPoles { wn, zeta } | Element::ComplexZeros { wn, zeta } => {
                    parameter(ui, "wn (rad/s)", wn, 1e-9..=1e9, &mut editing);
                    parameter(ui, "Damping", zeta, -MAX_ZETA..=MAX_ZETA, &mut editing);
                }
                Element::Lead { zero, pole } | Element::Lag { zero, pole } => {
                    parameter(
                        ui,
                        &format!("Zero at {unit} ="),
                        zero,
                        real_range.clone(),
                        &mut editing,
                    );
                    parameter(
                        ui,
                        &format!("Pole at {unit} ="),
                        pole,
                        real_range.clone(),
                        &mut editing,
                    );
                }
                Element::Notch { wn, depth, width } => {
                    parameter(ui, "wn (rad/s)", wn, 1e-9..=1e9, &mut editing);
                    parameter(ui, "Zero damping", depth, 0.0..=MAX_ZETA, &mut editing);
                    parameter(ui, "Pole damping", width, 1e-3..=MAX_ZETA, &mut editing);
                }
                Element::Integrator => {
                    crate::tokens::helper(
                        ui,
                        format!("Pole at {unit} = {}", if ts > 0. { 1 } else { 0 }),
                    );
                }
                Element::Differentiator => {
                    crate::tokens::helper(
                        ui,
                        format!("Zero at {unit} = {}", if ts > 0. { 1 } else { 0 }),
                    );
                }
            }
        }
        if let Some(i) = remove {
            self.compensator.elements.remove(i);
        }
        self.editing |= editing;
        ui.add_space(crate::tokens::S);
        let full = egui::vec2(ui.available_width(), crate::tokens::CONTROL);
        ui.add_enabled_ui(self.compensator.elements.len() < MAX_ELEMENTS, |ui| {
            ui.menu_button("Add element", |ui| {
                for kind in ElementKind::ALL {
                    if ui.button(kind.label()).clicked() {
                        self.add_element(kind, None);
                        ui.close();
                    }
                }
            });
        });
        ui.add_space(crate::tokens::XS);
        ui.horizontal(|ui| {
            let half = egui::vec2((full.x - crate::tokens::S) / 2., full.y);
            if ui
                .add_enabled(self.can_undo(), egui::Button::new("Undo").min_size(half))
                .clicked()
            {
                self.undo();
            }
            if ui
                .add_enabled(self.can_redo(), egui::Button::new("Redo").min_size(half))
                .clicked()
            {
                self.redo();
            }
        });
    }

    fn requirements_ui(&mut self, ui: &mut egui::Ui) {
        ui.label("Design requirements");
        let rows: [(&str, &str, &mut Limit, std::ops::RangeInclusive<f64>); 6] = [
            (
                "Settling time ≤",
                "s",
                &mut self.requirements.settling,
                1e-6..=1e6,
            ),
            (
                "Overshoot ≤",
                "%",
                &mut self.requirements.overshoot,
                1e-3..=99.9,
            ),
            ("Damping ≥", "", &mut self.requirements.damping, 0.0..=0.999),
            (
                "Natural freq. ≥",
                "rad/s",
                &mut self.requirements.frequency,
                1e-6..=1e6,
            ),
            (
                "Gain margin ≥",
                "dB",
                &mut self.requirements.gain_margin,
                0.0..=100.,
            ),
            (
                "Phase margin ≥",
                "°",
                &mut self.requirements.phase_margin,
                0.0..=180.,
            ),
        ];
        for (label, unit, limit, range) in rows {
            ui.horizontal(|ui| {
                ui.checkbox(&mut limit.on, label);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if !unit.is_empty() {
                        ui.label(unit);
                    }
                    let speed = (limit.value.abs() * 0.01).max(1e-3);
                    ui.add_enabled(
                        limit.on,
                        egui::DragValue::new(&mut limit.value)
                            .speed(speed)
                            .range(range)
                            .max_decimals(4),
                    );
                });
            });
        }
        crate::tokens::helper(
            ui,
            "Pole limits shade the root locus; margin limits show on the Bode.",
        );
    }

    // -----------------------------------------------------------------------
    // Main view

    fn views_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(crate::tokens::XS, crate::tokens::XS);
            let views = &mut self.views;
            let chips: [(&str, &mut bool); 5] = [
                ("Root locus", &mut views.root_locus),
                ("Bode", &mut views.bode),
                ("Nichols", &mut views.nichols),
                ("Step", &mut views.step),
                ("r to u", &mut views.effort),
            ];
            for (i, (label, on)) in chips.into_iter().enumerate() {
                if i == 3 {
                    ui.add_space(crate::tokens::S);
                }
                let chip = crate::tokens::toggle(ui, *on, label);
                if i == 0 {
                    crate::focus_areas::register(
                        ui,
                        &chip,
                        crate::focus_areas::Area::App(AppKind::ControlSystemDesigner),
                    );
                }
                if chip.clicked() {
                    *on = !*on;
                }
            }
        });
    }

    /// Bounds of the root locus: the open-loop roots, C's roots and the
    /// closed-loop poles with a margin, both axes at one scale for `size`.
    fn locus_bounds(&self, design: &Design, size: egui::Vec2) -> egui_plot::PlotBounds {
        let ts = design.ts;
        let mut pts: Vec<[f64; 2]> = design
            .plant_poles
            .iter()
            .chain(&design.plant_zeros)
            .chain(&design.closed)
            .copied()
            .chain(
                self.compensator
                    .markers(ts)
                    .into_iter()
                    .map(|m| [m.root.re, m.root.im]),
            )
            .filter(|p| p[0].is_finite() && p[1].is_finite())
            .collect();
        if ts > 0. {
            pts.extend([[-1., 0.], [1., 0.], [0., 1.], [0., -1.]]);
        }
        if pts.is_empty() {
            pts.push([0., 0.]);
        }
        let (mut x0, mut x1, mut y) = (f64::INFINITY, f64::NEG_INFINITY, 0_f64);
        for p in &pts {
            x0 = x0.min(p[0]);
            x1 = x1.max(p[0]);
            y = y.max(p[1].abs());
        }
        let span = (x1 - x0)
            .max(2. * y)
            .max(1e-6 * x0.abs().max(x1.abs()))
            .max(0.5);
        let pad = 0.2 * span;
        let (mut x0, mut x1, mut y) = (x0 - pad, x1 + pad, y + pad);
        // One scale on both axes, so circles read as circles.
        let (w, h) = (size.x.max(1.) as f64, size.y.max(1.) as f64);
        let per_px = ((x1 - x0) / w).max(2. * y / h);
        let cx = 0.5 * (x0 + x1);
        x0 = cx - 0.5 * per_px * w;
        x1 = cx + 0.5 * per_px * w;
        y = 0.5 * per_px * h;
        egui_plot::PlotBounds::from_min_max([x0, -y], [x1, y])
    }

    /// The forbidden pole regions as a texture over `bounds`.
    fn region_texture(
        &mut self,
        ctx: &egui::Context,
        bounds: egui_plot::PlotBounds,
        ts: f64,
        size: egui::Vec2,
    ) -> egui::TextureId {
        let w = (size.x / 2.).clamp(16., 480.) as usize;
        let h = (size.y / 2.).clamp(16., 360.) as usize;
        let color = crate::tokens::palette(ctx).warning;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for v in bounds.min().into_iter().chain(bounds.max()).chain([ts]) {
            v.to_bits().hash(&mut hasher);
        }
        format!("{:?}", self.requirements).hash(&mut hasher);
        (w, h, color).hash(&mut hasher);
        let key = hasher.finish();
        if let Some((k, texture)) = &self.regions
            && *k == key
        {
            return texture.id();
        }
        let [x0, y0] = bounds.min();
        let [x1, y1] = bounds.max();
        let shade = egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 56);
        let mut pixels = Vec::with_capacity(w * h);
        for j in 0..h {
            let y = y1 - (j as f64 + 0.5) / h as f64 * (y1 - y0);
            for i in 0..w {
                let x = x0 + (i as f64 + 0.5) / w as f64 * (x1 - x0);
                pixels.push(if self.requirements.forbidden([x, y], ts) {
                    shade
                } else {
                    egui::Color32::TRANSPARENT
                });
            }
        }
        let texture = ctx.load_texture(
            "control_system_designer_regions",
            egui::ColorImage::new([w, h], pixels),
            egui::TextureOptions::LINEAR,
        );
        let id = texture.id();
        self.regions = Some((key, texture));
        id
    }

    fn locus_ui(&mut self, ui: &mut egui::Ui, design: &Design, size: egui::Vec2) {
        let p = crate::tokens::palette(ui.ctx());
        caption_line(ui, "Root locus of C·G·H");
        let ts = design.ts;
        let size = egui::vec2(size.x, size.y.max(48.));
        let bounds = self
            .drag
            .as_ref()
            .and_then(|d| d.bounds(Editor::RootLocus))
            .unwrap_or_else(|| self.locus_bounds(design, self.locus_size));
        let region = self
            .requirements
            .pole_limits()
            .then(|| self.region_texture(ui.ctx(), bounds, ts, size));
        let markers = self.compensator.markers(ts);
        // While a closed-loop pole is dragged, the squares follow the gain.
        let squares = match &self.drag {
            Some(Drag::Gain { locus, gain, .. }) => {
                locus_points_at(locus, self.compensator.gain / gain)
            }
            _ => design.closed.clone(),
        };
        let response = chart("control_system_designer_locus", size, ("Real", "Imaginary")).show(
            ui,
            |plot_ui| {
                plot_ui.set_plot_bounds(bounds);
                if let Some(texture) = region {
                    let [x0, y0] = bounds.min();
                    let [x1, y1] = bounds.max();
                    plot_ui.image(egui_plot::PlotImage::new(
                        "Forbidden region",
                        texture,
                        egui_plot::PlotPoint::new(0.5 * (x0 + x1), 0.5 * (y0 + y1)),
                        egui::vec2((x1 - x0) as f32, (y1 - y0) as f32),
                    ));
                }
                if ts > 0. {
                    let circle: Vec<[f64; 2]> = (0..=128)
                        .map(|k| {
                            let a = 2. * PI * k as f64 / 128.;
                            [a.cos(), a.sin()]
                        })
                        .collect();
                    plot_ui.line(
                        egui_plot::Line::new("Unit circle", circle)
                            .color(p.text_muted)
                            .style(egui_plot::LineStyle::dashed_dense())
                            .width(1.),
                    );
                }
                for branch in &design.locus.branches {
                    for segment in segments(branch.iter().copied()) {
                        plot_ui.line(
                            egui_plot::Line::new("Locus", segment)
                                .color(trace())
                                .width(1.5),
                        );
                    }
                }
                roots(
                    plot_ui,
                    "G·H poles",
                    &design.plant_poles,
                    false,
                    p.text_muted,
                );
                roots(
                    plot_ui,
                    "G·H zeros",
                    &design.plant_zeros,
                    true,
                    p.text_muted,
                );
                let (zeros, poles): (Vec<Marker>, Vec<Marker>) =
                    markers.iter().copied().partition(|m| m.zero);
                let at = |list: &[Marker]| -> Vec<[f64; 2]> {
                    list.iter().map(|m| [m.root.re, m.root.im]).collect()
                };
                roots(plot_ui, "C poles", &at(&poles), false, p.accent);
                roots(plot_ui, "C zeros", &at(&zeros), true, p.accent);
                plot_ui.points(
                    egui_plot::Points::new("Closed-loop poles", squares)
                        .shape(egui_plot::MarkerShape::Square)
                        .filled(true)
                        .radius(4.)
                        .color(p.text),
                );
            },
        );
        self.locus_size = response.transform.frame().size();
        let transform = response.transform;
        let r = &response.response;
        let screen =
            |q: [f64; 2]| transform.position_from_point(&egui_plot::PlotPoint::new(q[0], q[1]));
        let handles: Vec<(Handle, [f64; 2])> = markers
            .iter()
            .filter_map(|m| m.handle.map(|h| (h, [m.root.re, m.root.im])))
            .collect();
        let pick = |pos: egui::Pos2| -> Option<Result<usize, Handle>> {
            let pole = nearest(design.closed.iter().map(|q| screen(*q)), pos);
            let root = nearest(handles.iter().map(|(_, q)| screen(*q)), pos);
            match (pole, root) {
                (Some((i, a)), Some((j, b))) => {
                    Some(if b <= a { Err(handles[j].0) } else { Ok(i) })
                }
                (Some((i, _)), None) => Some(Ok(i)),
                (None, Some((j, _))) => Some(Err(handles[j].0)),
                (None, None) => None,
            }
        };
        if r.drag_started()
            && let Some(pos) = r.interact_pointer_pos()
        {
            self.drag = match pick(pos) {
                Some(Ok(_)) => Some(Drag::Gain {
                    locus: design.locus.clone(),
                    gain: design.compensator.gain,
                    bounds,
                }),
                Some(Err(handle)) => Some(Drag::Root { handle, bounds }),
                None => None,
            };
        }
        if r.dragged()
            && let Some(pos) = r.interact_pointer_pos()
        {
            let q = transform.value_from_position(pos);
            let scale = transform.dpos_dvalue().map(f64::abs);
            match self.drag.clone() {
                Some(Drag::Gain { locus, gain, .. }) => {
                    if let Some(k) = dragged_gain(&locus, gain, [q.x, q.y], scale) {
                        self.compensator.gain = k;
                    }
                }
                Some(Drag::Root { handle, .. }) => {
                    self.compensator.move_root(handle, [q.x, q.y], ts);
                }
                _ => {}
            }
        }
        if r.drag_stopped() && matches!(self.drag, Some(Drag::Gain { .. } | Drag::Root { .. })) {
            self.drag = None;
        }
        if r.hovered()
            && self.drag.is_none()
            && let Some(pos) = r.hover_pos()
            && pick(pos).is_some()
        {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
        if r.secondary_clicked()
            && let Some(pos) = r.interact_pointer_pos()
        {
            let q = transform.value_from_position(pos);
            self.menu_at = [q.x, q.y];
        }
        let mut add = None;
        r.context_menu(|ui| {
            for kind in [
                ElementKind::RealPole,
                ElementKind::RealZero,
                ElementKind::ComplexPoles,
                ElementKind::ComplexZeros,
            ] {
                if ui
                    .button(format!("Add {} here", kind.label().to_lowercase()))
                    .clicked()
                {
                    add = Some(kind);
                    ui.close();
                }
            }
        });
        if let Some(kind) = add {
            self.add_element(kind, Some(self.menu_at));
        }
    }

    fn bode_ui(&mut self, ui: &mut egui::Ui, design: &Design, size: egui::Vec2) {
        let p = crate::tokens::palette(ui.ctx());
        caption_line(
            ui,
            format!(
                "Bode of C·G·H · GM {} · PM {}",
                if design.gain_margin.is_finite() && design.gain_margin > 0. {
                    format!("{} dB", short(20. * design.gain_margin.log10()))
                } else {
                    short(design.gain_margin)
                },
                if design.phase_margin.is_finite() {
                    format!("{}°", short(design.phase_margin))
                } else {
                    short(design.phase_margin)
                }
            ),
        );
        let ts = design.ts;
        let half = ((size.y - crate::tokens::S) / 2.).max(40.);
        // C's poles and zeros at their corner frequencies on the magnitude.
        let marks: Vec<(Handle, [f64; 2], bool)> = self
            .compensator
            .handles()
            .into_iter()
            .filter_map(|h| {
                let x = self.compensator.corner(h, ts)?.log10();
                interpolate(&design.magnitude, x).map(|y| (h, [x, y], h.zero))
            })
            .collect();
        let frozen = self.drag.as_ref().and_then(|d| d.bounds(Editor::Bode));
        let gm_line = self.requirements.gain_margin.get();
        let pm_line = self.requirements.phase_margin.get();
        let wcg = design.phase_crossover.log10();
        let wcp = design.gain_crossover.log10();
        let response = chart(
            "control_system_designer_bode_magnitude",
            egui::vec2(size.x, half),
            ("", "Magnitude (dB)"),
        )
        .x_axis_formatter(|mark, _| short(10_f64.powf(mark.value)))
        .show(ui, |plot_ui| {
            // A drag holds the view still; otherwise the view fits the curve
            // (egui_plot remembers set bounds, so auto bounds come back here).
            match frozen {
                Some(bounds) => plot_ui.set_plot_bounds(bounds),
                None => plot_ui.set_auto_bounds(true),
            }
            plot_ui.hline(
                egui_plot::HLine::new("0 dB", 0.)
                    .color(p.hairline)
                    .width(1.),
            );
            if let Some(g) = gm_line {
                plot_ui.hline(
                    egui_plot::HLine::new("Gain margin limit", -g)
                        .color(p.warning)
                        .style(egui_plot::LineStyle::dashed_dense())
                        .width(1.),
                );
            }
            if wcg.is_finite()
                && let Some(y) = interpolate(&design.magnitude, wcg)
            {
                plot_ui.line(
                    egui_plot::Line::new("Gain margin", vec![[wcg, y], [wcg, 0.]])
                        .color(p.text_muted)
                        .width(1.5),
                );
            }
            plot_ui.line(
                egui_plot::Line::new("Magnitude", design.magnitude.clone())
                    .color(trace())
                    .width(1.5),
            );
            corner_marks(plot_ui, &marks, p.accent);
        });
        let transform = response.transform;
        let r = &response.response;
        let screen =
            |q: [f64; 2]| transform.position_from_point(&egui_plot::PlotPoint::new(q[0], q[1]));
        let pick =
            |pos| nearest(marks.iter().map(|(_, q, _)| screen(*q)), pos).map(|(i, _)| marks[i].0);
        if r.drag_started()
            && let Some(pos) = r.interact_pointer_pos()
        {
            let bounds = *transform.bounds();
            self.drag = Some(match pick(pos) {
                Some(handle) => Drag::Corner { handle, bounds },
                None => Drag::Level {
                    editor: Editor::Bode,
                    start: transform.value_from_position(pos).y,
                    gain: self.compensator.gain,
                    bounds,
                },
            });
        }
        if r.dragged()
            && let Some(pos) = r.interact_pointer_pos()
        {
            let q = transform.value_from_position(pos);
            match self.drag.clone() {
                Some(Drag::Corner { handle, .. }) => {
                    self.compensator.move_corner(handle, 10_f64.powf(q.x), ts);
                }
                Some(Drag::Level {
                    editor: Editor::Bode,
                    start,
                    gain,
                    ..
                }) => self.compensator.gain = gain * 10_f64.powf((q.y - start) / 20.),
                _ => {}
            }
        }
        if r.drag_stopped()
            && matches!(
                self.drag,
                Some(
                    Drag::Corner { .. }
                        | Drag::Level {
                            editor: Editor::Bode,
                            ..
                        }
                )
            )
        {
            self.drag = None;
        }
        if r.hovered()
            && self.drag.is_none()
            && let Some(pos) = r.hover_pos()
        {
            ui.ctx().set_cursor_icon(if pick(pos).is_some() {
                egui::CursorIcon::ResizeHorizontal
            } else {
                egui::CursorIcon::ResizeVertical
            });
        }
        ui.add_space(crate::tokens::S);
        // The -180 line nearest the phase at the gain crossover.
        let base = if wcp.is_finite() {
            interpolate(&design.phase, wcp)
                .map_or(-180., |ph| -180. + 360. * ((ph + 180.) / 360.).round())
        } else {
            -180.
        };
        chart(
            "control_system_designer_bode_phase",
            egui::vec2(size.x, half),
            ("Frequency (rad/s)", "Phase (deg)"),
        )
        .x_axis_formatter(|mark, _| short(10_f64.powf(mark.value)))
        .show(ui, |plot_ui| {
            plot_ui.hline(
                egui_plot::HLine::new("-180°", base)
                    .color(p.hairline)
                    .width(1.),
            );
            if let Some(m) = pm_line {
                plot_ui.hline(
                    egui_plot::HLine::new("Phase margin limit", base + m)
                        .color(p.warning)
                        .style(egui_plot::LineStyle::dashed_dense())
                        .width(1.),
                );
            }
            if wcp.is_finite()
                && let Some(y) = interpolate(&design.phase, wcp)
            {
                plot_ui.line(
                    egui_plot::Line::new("Phase margin", vec![[wcp, base], [wcp, y]])
                        .color(p.text_muted)
                        .width(1.5),
                );
            }
            plot_ui.line(
                egui_plot::Line::new("Phase", design.phase.clone())
                    .color(trace())
                    .width(1.5),
            );
        });
    }

    fn nichols_ui(&mut self, ui: &mut egui::Ui, design: &Design, size: egui::Vec2) {
        let p = crate::tokens::palette(ui.ctx());
        caption_line(ui, "Nichols of C·G·H with ngrid");
        let Some(nichols) = &design.nichols else {
            caption_line(ui, "Computing…");
            return;
        };
        let size = egui::vec2(size.x, size.y.max(48.));
        let bounds = self
            .drag
            .as_ref()
            .and_then(|d| d.bounds(Editor::Nichols))
            .unwrap_or_else(|| {
                egui_plot::PlotBounds::from_min_max(
                    [nichols.x[0], nichols.y[0]],
                    [nichols.x[1], nichols.y[1]],
                )
            });
        let response = chart(
            "control_system_designer_nichols",
            size,
            ("Phase (deg)", "Magnitude (dB)"),
        )
        .show(ui, |plot_ui| {
            if bounds.is_valid() {
                plot_ui.set_plot_bounds(bounds);
            } else {
                plot_ui.set_auto_bounds(true);
            }
            for contour in &nichols.grid {
                plot_ui.line(
                    egui_plot::Line::new("ngrid", contour.clone())
                        .color(p.hairline)
                        .width(1.),
                );
            }
            let [x0, x1] = nichols.x;
            let critical: Vec<[f64; 2]> = ((x0 / 360.).floor() as i64..=(x1 / 360.).ceil() as i64)
                .map(|k| [-180. + 360. * k as f64, 0.])
                .filter(|q| q[0] >= x0 && q[0] <= x1)
                .collect();
            plot_ui.points(
                egui_plot::Points::new("-1 point", critical)
                    .shape(egui_plot::MarkerShape::Plus)
                    .radius(5.)
                    .color(p.text_muted),
            );
            plot_ui.line(
                egui_plot::Line::new("C·G·H", nichols.curve.clone())
                    .color(trace())
                    .width(1.5),
            );
        });
        let transform = response.transform;
        let r = &response.response;
        if r.drag_started()
            && let Some(pos) = r.interact_pointer_pos()
        {
            self.drag = Some(Drag::Level {
                editor: Editor::Nichols,
                start: transform.value_from_position(pos).y,
                gain: self.compensator.gain,
                bounds: *transform.bounds(),
            });
        }
        if r.dragged()
            && let Some(pos) = r.interact_pointer_pos()
            && let Some(Drag::Level {
                editor: Editor::Nichols,
                start,
                gain,
                ..
            }) = self.drag.clone()
        {
            let q = transform.value_from_position(pos);
            self.compensator.gain = gain * 10_f64.powf((q.y - start) / 20.);
        }
        if r.drag_stopped()
            && matches!(
                self.drag,
                Some(Drag::Level {
                    editor: Editor::Nichols,
                    ..
                })
            )
        {
            self.drag = None;
        }
        if r.hovered() && self.drag.is_none() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
        }
    }
}

fn step_response_ui(ui: &mut egui::Ui, design: &Design, height: f32) {
    let p = crate::tokens::palette(ui.ctx());
    caption_line(
        ui,
        format!(
            "Step r to y · rise {} s · settling {} s · overshoot {} % · peak {}",
            short(design.rise),
            short(design.settling),
            short(design.overshoot),
            short(design.peak)
        ),
    );
    chart(
        "control_system_designer_step",
        egui::vec2(ui.available_width(), height),
        ("Time (s)", "Output y"),
    )
    .show(ui, |plot_ui| {
        if design.settling.is_finite() {
            plot_ui.vline(
                egui_plot::VLine::new("Settling time", design.settling)
                    .color(p.text_muted)
                    .style(egui_plot::LineStyle::dashed_dense())
                    .width(1.),
            );
        }
        plot_ui.line(
            egui_plot::Line::new("y", design.step.clone())
                .color(trace())
                .width(1.5),
        );
        if design.peak.is_finite() && design.peak_time.is_finite() {
            plot_ui.points(
                egui_plot::Points::new("Peak", vec![[design.peak_time, design.peak]])
                    .shape(egui_plot::MarkerShape::Circle)
                    .filled(true)
                    .radius(3.)
                    .color(p.text),
            );
        }
    });
}

fn control_effort_ui(ui: &mut egui::Ui, design: &Design, height: f32) {
    caption_line(ui, "Step r to u (plant input)");
    match &design.effort {
        Some(effort) => {
            chart(
                "control_system_designer_effort",
                egui::vec2(ui.available_width(), height),
                ("Time (s)", "Input u"),
            )
            .show(ui, |plot_ui| {
                plot_ui.line(
                    egui_plot::Line::new("u", effort.clone())
                        .color(trace())
                        .width(1.5),
                );
            });
        }
        None => {
            crate::tokens::helper(ui, "The r to u step has no finite answer for this loop.");
        }
    }
}

/// One muted line above a plot, cut with an ellipsis (full text on hover)
/// so the plots below keep their height.
fn caption_line(ui: &mut egui::Ui, text: impl Into<String>) {
    let p = crate::tokens::palette(ui.ctx());
    ui.add(
        egui::Label::new(
            egui::RichText::new(text.into())
                .size(crate::tokens::BODY)
                .color(p.text_muted),
        )
        .truncate(),
    );
}

/// Points of every branch where the locus gain is `k` (interpolated).
fn locus_points_at(locus: &Locus, k: f64) -> Vec<[f64; 2]> {
    let j = locus.gains.partition_point(|g| *g < k);
    locus
        .branches
        .iter()
        .filter_map(|branch| {
            let (a, b) = (j.checked_sub(1)?, j);
            let (pa, pb) = (branch.get(a)?, branch.get(b)?);
            let (ka, kb) = (locus.gains[a], locus.gains[b]);
            let t = if kb > ka { (k - ka) / (kb - ka) } else { 0. };
            let q = [pa[0] + t * (pb[0] - pa[0]), pa[1] + t * (pb[1] - pa[1])];
            (q[0].is_finite() && q[1].is_finite()).then_some(q)
        })
        .collect()
}

fn chart<'a>(id: &str, size: egui::Vec2, (x_label, y_label): (&str, &str)) -> egui_plot::Plot<'a> {
    egui_plot::Plot::new(id.to_owned())
        .width(size.x)
        .height(size.y)
        .allow_scroll(false)
        .allow_drag(false)
        .allow_zoom(false)
        .allow_boxed_zoom(false)
        .allow_axis_zoom_drag(false)
        .allow_double_click_reset(false)
        .auto_bounds(egui::Vec2b::TRUE)
        .x_axis_label(x_label.to_owned())
        .y_axis_label(y_label.to_owned())
        .show_background(false)
}
fn trace() -> egui::Color32 {
    let [r, g, b] = matrust::plot::COLORS[0];
    egui::Color32::from_rgb(r, g, b)
}
/// Poles as x, zeros as o.
fn roots(
    plot_ui: &mut egui_plot::PlotUi<'_>,
    name: &str,
    points: &[[f64; 2]],
    zero: bool,
    color: egui::Color32,
) {
    if points.is_empty() {
        return;
    }
    plot_ui.points(
        egui_plot::Points::new(name.to_owned(), points.to_vec())
            .shape(if zero {
                egui_plot::MarkerShape::Circle
            } else {
                egui_plot::MarkerShape::Cross
            })
            .filled(false)
            .radius(5.)
            .color(color),
    );
}
fn corner_marks(
    plot_ui: &mut egui_plot::PlotUi<'_>,
    marks: &[(Handle, [f64; 2], bool)],
    color: egui::Color32,
) {
    let (zeros, poles): (Vec<_>, Vec<_>) = marks.iter().partition(|m| m.2);
    let at = |list: Vec<&(Handle, [f64; 2], bool)>| -> Vec<[f64; 2]> {
        list.into_iter().map(|m| m.1).collect()
    };
    roots(plot_ui, "C poles", &at(poles), false, color);
    roots(plot_ui, "C zeros", &at(zeros), true, color);
}
/// The point nearest `pos` within the pick radius: index and distance.
fn nearest(points: impl Iterator<Item = egui::Pos2>, pos: egui::Pos2) -> Option<(usize, f32)> {
    points
        .enumerate()
        .map(|(i, p)| (i, p.distance(pos)))
        .filter(|(_, d)| d.is_finite() && *d <= HIT)
        .min_by(|a, b| a.1.total_cmp(&b.1))
}
