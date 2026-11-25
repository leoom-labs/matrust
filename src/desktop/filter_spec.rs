//! Filter Designer model: what the user specified, which fields each method
//! uses, plain validation messages, the spec mask and the MATLAB script that
//! builds the filter. The script is the single source of truth: the preview,
//! workspace export, Apply and the block model all run it, so what the user
//! copies rebuilds exactly the filter they see.
use std::fmt::Write as _;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Response {
    Lowpass,
    Highpass,
    Bandpass,
    Bandstop,
    Notch,
    Differentiator,
    Hilbert,
    Multiband,
    Arbitrary,
}
pub const RESPONSES: [Response; 9] = [
    Response::Lowpass,
    Response::Highpass,
    Response::Bandpass,
    Response::Bandstop,
    Response::Notch,
    Response::Differentiator,
    Response::Hilbert,
    Response::Multiband,
    Response::Arbitrary,
];
impl Response {
    pub fn label(self) -> &'static str {
        match self {
            Self::Lowpass => "Lowpass",
            Self::Highpass => "Highpass",
            Self::Bandpass => "Bandpass",
            Self::Bandstop => "Bandstop",
            Self::Notch => "Notch",
            Self::Differentiator => "Differentiator",
            Self::Hilbert => "Hilbert transformer",
            Self::Multiband => "Multiband",
            Self::Arbitrary => "Arbitrary magnitude",
        }
    }
    /// Lowpass/highpass/bandpass/bandstop: the classic two-level specs.
    pub fn classic(self) -> bool {
        matches!(
            self,
            Self::Lowpass | Self::Highpass | Self::Bandpass | Self::Bandstop
        )
    }
    pub fn two_edges(self) -> bool {
        matches!(self, Self::Bandpass | Self::Bandstop | Self::Hilbert)
    }
    /// The type argument of butter/cheby1/cheby2/ellip/fir1.
    fn matlab_type(self) -> &'static str {
        match self {
            Self::Highpass => "high",
            Self::Bandpass => "bandpass",
            Self::Bandstop => "stop",
            _ => "low",
        }
    }
    pub fn methods(self) -> &'static [Method] {
        use Method::*;
        match self {
            Self::Lowpass => &[
                Butterworth,
                Chebyshev1,
                Chebyshev2,
                Elliptic,
                Bessel,
                Window,
                Equiripple,
                LeastSquares,
                FrequencySampling,
            ],
            Self::Highpass | Self::Bandpass | Self::Bandstop => &[
                Butterworth,
                Chebyshev1,
                Chebyshev2,
                Elliptic,
                Window,
                Equiripple,
                LeastSquares,
                FrequencySampling,
            ],
            Self::Notch => &[NotchIir],
            Self::Differentiator | Self::Hilbert => &[Equiripple, LeastSquares],
            Self::Multiband => &[Equiripple, LeastSquares, FrequencySampling],
            Self::Arbitrary => &[FrequencySampling, LeastSquares, Equiripple],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Butterworth,
    Chebyshev1,
    Chebyshev2,
    Elliptic,
    Bessel,
    NotchIir,
    Window,
    Equiripple,
    LeastSquares,
    FrequencySampling,
}
impl Method {
    pub fn label(self) -> &'static str {
        match self {
            Self::Butterworth => "Butterworth",
            Self::Chebyshev1 => "Chebyshev I",
            Self::Chebyshev2 => "Chebyshev II",
            Self::Elliptic => "Elliptic",
            Self::Bessel => "Bessel",
            Self::NotchIir => "Second-order notch",
            Self::Window => "Window",
            Self::Equiripple => "Equiripple",
            Self::LeastSquares => "Least-squares",
            Self::FrequencySampling => "Frequency sampling",
        }
    }
    pub fn iir(self) -> bool {
        matches!(
            self,
            Self::Butterworth
                | Self::Chebyshev1
                | Self::Chebyshev2
                | Self::Elliptic
                | Self::Bessel
                | Self::NotchIir
        )
    }
    /// The MATLAB function the method calls, shown next to the method.
    pub fn function(self) -> &'static str {
        match self {
            Self::Butterworth => "butter",
            Self::Chebyshev1 => "cheby1",
            Self::Chebyshev2 => "cheby2",
            Self::Elliptic => "ellip",
            Self::Bessel => "besself + bilinear",
            Self::NotchIir => "iirnotch",
            Self::Window => "fir1",
            Self::Equiripple => "firpm",
            Self::LeastSquares => "firls",
            Self::FrequencySampling => "fir2",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Units {
    Hz,
    /// ×π rad/sample: 1 is the Nyquist frequency.
    Normalized,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Structure {
    Direct,
    Sos,
}

/// FIR windows: label and MATLAB function.
pub const WINDOWS: [(&str, &str); 10] = [
    ("Hamming", "hamming"),
    ("Hann", "hann"),
    ("Blackman", "blackman"),
    ("Kaiser", "kaiser"),
    ("Bartlett", "bartlett"),
    ("Triangular", "triang"),
    ("Rectangular", "rectwin"),
    ("Blackman-Harris", "blackmanharris"),
    ("Nuttall", "nuttallwin"),
    ("Flat top", "flattopwin"),
];
pub const KAISER: usize = 3;

/// Every input the designer can show; `Spec::fields` lists the visible ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Field {
    Order,
    Fs,
    Fc,
    Fpass,
    Fstop,
    Apass,
    Astop,
    Wpass,
    Wstop,
    Window,
    Beta,
    F0,
    Bandwidth,
    Freqs,
    Amps,
    Weights,
    Structure,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Spec {
    pub response: Response,
    pub method: Method,
    pub minimum: bool,
    pub order: usize,
    pub units: Units,
    /// Sample rate in Hz (used when units are Hz).
    pub fs: f64,
    /// Frequencies are in the current units.
    pub fc: [f64; 2],
    pub fpass: [f64; 2],
    pub fstop: [f64; 2],
    pub apass: f64,
    pub astop: f64,
    pub wpass: f64,
    pub wstop: f64,
    /// Index into `WINDOWS`.
    pub window: usize,
    pub beta: f64,
    pub f0: f64,
    pub bandwidth: f64,
    pub freqs: String,
    pub amps: String,
    pub weights: String,
    pub structure: Structure,
}
impl Default for Spec {
    fn default() -> Self {
        let mut spec = Self {
            response: Response::Lowpass,
            method: Method::Equiripple,
            minimum: true,
            order: 20,
            units: Units::Hz,
            fs: 48000.,
            fc: [0.; 2],
            fpass: [0.; 2],
            fstop: [0.; 2],
            apass: 1.,
            astop: 80.,
            wpass: 1.,
            wstop: 1.,
            window: 0,
            beta: 0.5,
            f0: 0.,
            bandwidth: 0.,
            freqs: String::new(),
            amps: String::new(),
            weights: String::new(),
            structure: Structure::Sos,
        };
        spec.reset_frequencies();
        spec
    }
}

impl Spec {
    /// The method can compute its own order from the specs.
    pub fn minimum_available(&self) -> bool {
        self.response.classic()
            && matches!(
                self.method,
                Method::Butterworth
                    | Method::Chebyshev1
                    | Method::Chebyshev2
                    | Method::Elliptic
                    | Method::Window
                    | Method::Equiripple
            )
    }
    pub fn uses_minimum(&self) -> bool {
        self.minimum && self.minimum_available()
    }
    pub fn nyquist(&self) -> f64 {
        match self.units {
            Units::Hz => self.fs / 2.,
            Units::Normalized => 1.,
        }
    }
    /// Normalized frequency (1 = Nyquist) of a value in the current units.
    pub fn norm(&self, f: f64) -> f64 {
        f / self.nyquist()
    }
    /// Choose a response; keeps the method if it applies, else the first one,
    /// and resets the frequencies to that response's defaults.
    pub fn set_response(&mut self, response: Response) {
        if response == self.response {
            return;
        }
        self.response = response;
        if !response.methods().contains(&self.method) {
            self.method = response.methods()[0];
        }
        self.reset_frequencies();
    }
    pub fn set_method(&mut self, method: Method) {
        if self.response.methods().contains(&method) {
            self.method = method;
        }
    }
    /// Which families (IIR, FIR) the response offers, in that order.
    pub fn families(&self) -> (bool, bool) {
        let m = self.response.methods();
        (m.iter().any(|m| m.iir()), m.iter().any(|m| !m.iir()))
    }
    /// Switch to the first method of the other family; no-op when the
    /// family is already selected or the response has none of it.
    pub fn set_family(&mut self, iir: bool) {
        if self.method.iir() == iir {
            return;
        }
        if let Some(&m) = self.response.methods().iter().find(|m| m.iir() == iir) {
            self.set_method(m);
        }
    }
    pub fn set_units(&mut self, units: Units) {
        if units == self.units || !(self.fs.is_finite() && self.fs > 0.) {
            return;
        }
        let scale = match units {
            Units::Hz => self.fs / 2.,
            Units::Normalized => 2. / self.fs,
        };
        for f in self
            .fc
            .iter_mut()
            .chain(&mut self.fpass)
            .chain(&mut self.fstop)
            .chain([&mut self.f0, &mut self.bandwidth])
        {
            *f *= scale;
        }
        if let Ok(values) = parse_vector(&self.freqs) {
            self.freqs = format_vector(&values.iter().map(|v| v * scale).collect::<Vec<_>>());
        }
        self.units = units;
    }
    /// Defaults as fractions of Nyquist, in the current units.
    pub fn reset_frequencies(&mut self) {
        let n = self.nyquist();
        let at = |x: f64| x * n;
        match self.response {
            Response::Lowpass => {
                self.fc = [at(0.45), 0.];
                self.fpass = [at(0.4), 0.];
                self.fstop = [at(0.5), 0.];
            }
            Response::Highpass => {
                self.fc = [at(0.45), 0.];
                self.fpass = [at(0.5), 0.];
                self.fstop = [at(0.4), 0.];
            }
            Response::Bandpass => {
                self.fc = [at(0.35), at(0.55)];
                self.fpass = [at(0.35), at(0.55)];
                self.fstop = [at(0.3), at(0.6)];
            }
            Response::Bandstop => {
                self.fc = [at(0.35), at(0.55)];
                self.fpass = [at(0.3), at(0.6)];
                self.fstop = [at(0.35), at(0.55)];
            }
            Response::Notch => {
                self.f0 = at(0.1);
                self.bandwidth = at(0.01);
            }
            Response::Differentiator => self.fpass = [at(0.9), 0.],
            Response::Hilbert => self.fpass = [at(0.05), at(0.95)],
            Response::Multiband => {
                self.freqs = format_vector(&[0., at(0.2), at(0.25), at(0.5), at(0.55), at(1.)]);
                self.amps = "1 1 0 0 0.5 0.5".into();
                self.weights = "1 1 1".into();
            }
            Response::Arbitrary => {
                self.freqs = format_vector(&[0., at(0.2), at(0.4), at(0.6), at(0.8), at(1.)]);
                self.amps = "1 0.8 0.6 0.4 0.2 0".into();
                self.weights = String::new();
            }
        }
    }

    /// The inputs this response + method + order mode uses, in display order.
    pub fn fields(&self) -> Vec<Field> {
        use Field::*;
        let mut f = Vec::new();
        if self.method != Method::NotchIir && !self.uses_minimum() {
            f.push(Order);
        }
        if self.units == Units::Hz {
            f.push(Fs);
        }
        let min = self.uses_minimum();
        match self.method {
            _ if min => f.extend([Fpass, Fstop, Apass, Astop]),
            Method::Butterworth | Method::Bessel => f.push(Fc),
            Method::Chebyshev1 => f.extend([Fpass, Apass]),
            Method::Chebyshev2 => f.extend([Fstop, Astop]),
            Method::Elliptic => f.extend([Fpass, Apass, Astop]),
            Method::NotchIir => f.extend([F0, Bandwidth]),
            Method::Window => {
                f.extend([Fc, Window]);
                if self.window == KAISER {
                    f.push(Beta);
                }
            }
            Method::Equiripple | Method::LeastSquares | Method::FrequencySampling => {
                match self.response {
                    Response::Differentiator | Response::Hilbert => f.push(Fpass),
                    Response::Multiband | Response::Arbitrary => {
                        f.extend([Freqs, Amps]);
                        if self.method != Method::FrequencySampling {
                            f.push(Weights);
                        }
                    }
                    _ => {
                        f.extend([Fpass, Fstop]);
                        if self.method != Method::FrequencySampling {
                            f.extend([Wpass, Wstop]);
                        }
                    }
                }
            }
        }
        if self.method.iir() {
            f.push(Structure);
        }
        f
    }

    /// Plain messages, one per offending field; empty when the spec designs.
    pub fn problems(&self) -> Vec<(Field, String)> {
        let mut out = Vec::new();
        let fields = self.fields();
        let has = |x: Field| fields.contains(&x);
        let unit = match self.units {
            Units::Hz => " Hz",
            Units::Normalized => "",
        };
        let nyq = self.nyquist();
        if has(Field::Fs) && !(self.fs.is_finite() && self.fs > 0.) {
            out.push((Field::Fs, "Sample rate must be a positive number".into()));
            return out;
        }
        if has(Field::Order) {
            let (lo, hi) = match self.method {
                Method::Bessel => (1, 25),
                m if m.iir() => (1, 40),
                _ => (2, 2000),
            };
            if !(lo..=hi).contains(&self.order) {
                out.push((Field::Order, format!("Order must be {lo} to {hi}")));
            } else if !self.method.iir()
                && self.order % 2 == 1
                && matches!(self.response, Response::Highpass | Response::Bandstop)
            {
                out.push((
                    Field::Order,
                    "Highpass and bandstop FIR filters need an even order".into(),
                ));
            } else if !self.method.iir()
                && self.order % 2 == 1
                && matches!(self.response, Response::Multiband | Response::Arbitrary)
                && self.end_gain() != 0.
            {
                out.push((
                    Field::Order,
                    "An odd order forces zero gain at Nyquist; use an even order".into(),
                ));
            }
        }
        let inside = |f: f64| f.is_finite() && f > 0. && f < nyq;
        let range = format!("between 0 and {}{unit}", fmt(nyq));
        let two = self.response.two_edges();
        let count = if two { 2 } else { 1 };
        for (field, values) in [
            (Field::Fc, &self.fc),
            (Field::Fpass, &self.fpass),
            (Field::Fstop, &self.fstop),
        ] {
            if !has(field) {
                continue;
            }
            let v = &values[..count];
            if v.iter().any(|&f| !inside(f)) {
                out.push((field, format!("Must lie {range}")));
            } else if two && v[0] >= v[1] {
                out.push((field, "The first edge must be below the second".into()));
            }
        }
        if out.is_empty() && has(Field::Fpass) && has(Field::Fstop) {
            let (p, s) = (self.fpass, self.fstop);
            let message = match self.response {
                Response::Lowpass if p[0] >= s[0] => Some("Fpass must be below Fstop"),
                Response::Highpass if s[0] >= p[0] => Some("Fstop must be below Fpass"),
                Response::Bandpass if !(s[0] < p[0] && p[1] < s[1]) => {
                    Some("Bandpass needs Fstop1 < Fpass1 < Fpass2 < Fstop2")
                }
                Response::Bandstop if !(p[0] < s[0] && s[1] < p[1]) => {
                    Some("Bandstop needs Fpass1 < Fstop1 < Fstop2 < Fpass2")
                }
                _ => None,
            };
            if let Some(message) = message {
                out.push((Field::Fstop, message.into()));
            }
        }
        if has(Field::Apass) && !(self.apass.is_finite() && self.apass > 0. && self.apass < 20.) {
            out.push((Field::Apass, "Apass must be between 0 and 20 dB".into()));
        }
        if has(Field::Astop) && !(self.astop.is_finite() && self.astop > 0. && self.astop <= 200.) {
            out.push((Field::Astop, "Astop must be between 0 and 200 dB".into()));
        } else if has(Field::Apass) && has(Field::Astop) && self.astop <= self.apass {
            out.push((Field::Astop, "Astop must be larger than Apass".into()));
        }
        for (field, w) in [(Field::Wpass, self.wpass), (Field::Wstop, self.wstop)] {
            if has(field) && !(w.is_finite() && w > 0.) {
                out.push((field, "Weight must be a positive number".into()));
            }
        }
        if has(Field::Beta) && !(self.beta.is_finite() && (0. ..=50.).contains(&self.beta)) {
            out.push((Field::Beta, "Kaiser β must be between 0 and 50".into()));
        }
        if has(Field::F0) && !inside(self.f0) {
            out.push((Field::F0, format!("Notch frequency must lie {range}")));
        }
        if has(Field::Bandwidth)
            && !(self.bandwidth.is_finite() && self.bandwidth > 0. && self.bandwidth < nyq)
        {
            out.push((Field::Bandwidth, format!("Bandwidth must lie {range}")));
        } else if has(Field::F0)
            && inside(self.f0)
            && self.bandwidth >= 2. * self.f0.min(nyq - self.f0)
        {
            out.push((
                Field::Bandwidth,
                "Bandwidth is too wide for this notch frequency".into(),
            ));
        }
        if has(Field::Freqs) {
            self.vector_problems(&mut out, nyq, unit);
        }
        out
    }
    fn end_gain(&self) -> f64 {
        parse_vector(&self.amps)
            .ok()
            .and_then(|a| a.last().copied())
            .unwrap_or(0.)
    }
    fn vector_problems(&self, out: &mut Vec<(Field, String)>, nyq: f64, unit: &str) {
        let freqs = match parse_vector(&self.freqs) {
            Ok(f) => f,
            Err(e) => return out.push((Field::Freqs, e)),
        };
        let amps = match parse_vector(&self.amps) {
            Ok(a) => a,
            Err(e) => return out.push((Field::Amps, e)),
        };
        let paired = self.method != Method::FrequencySampling;
        if freqs.len() < 2 || (paired && freqs.len() % 2 == 1) {
            return out.push((
                Field::Freqs,
                if paired {
                    "Give band edges in pairs: an even number of frequencies".into()
                } else {
                    "Give at least two frequencies".into()
                },
            ));
        }
        if freqs
            .iter()
            .any(|&f| !(f.is_finite() && (0. ..=nyq).contains(&f)))
            || freqs.windows(2).any(|w| w[1] < w[0])
        {
            return out.push((
                Field::Freqs,
                format!("Frequencies must rise from 0 to {}{unit}", fmt(nyq)),
            ));
        }
        if !paired && (freqs[0] != 0. || *freqs.last().unwrap() != nyq) {
            return out.push((
                Field::Freqs,
                format!(
                    "Frequency sampling needs the first point at 0 and the last at {}{unit}",
                    fmt(nyq)
                ),
            ));
        }
        if paired && freqs.chunks(2).any(|band| band[1] <= band[0]) {
            return out.push((
                Field::Freqs,
                "Each band needs its upper edge above its lower edge".into(),
            ));
        }
        if amps.len() != freqs.len() || amps.iter().any(|a| !a.is_finite() || *a < 0.) {
            return out.push((
                Field::Amps,
                format!("Give {} non-negative gains, one per frequency", freqs.len()),
            ));
        }
        if self.fields().contains(&Field::Weights) && !self.weights.trim().is_empty() {
            match parse_vector(&self.weights) {
                Ok(w)
                    if w.len() == freqs.len() / 2 && w.iter().all(|w| w.is_finite() && *w > 0.) => {
                }
                Ok(_) => out.push((
                    Field::Weights,
                    format!("Give {} positive weights, one per band", freqs.len() / 2),
                )),
                Err(e) => out.push((Field::Weights, e)),
            }
        }
    }

    /// Spec mask segments in normalized frequency: (f0, f1, upper dB, lower dB).
    /// Passbands span 0 … −Apass, stopbands −Astop … floor.
    pub fn mask(&self) -> Vec<MaskBand> {
        let has = |x: Field| self.fields().contains(&x);
        if !(has(Field::Fpass) && has(Field::Fstop)) || !self.response.classic() {
            return Vec::new();
        }
        let (p, s) = (
            self.fpass.map(|f| self.norm(f)),
            self.fstop.map(|f| self.norm(f)),
        );
        let apass = has(Field::Apass).then_some(self.apass);
        let astop = has(Field::Astop).then_some(self.astop);
        let pass = |f0, f1| MaskBand {
            f0,
            f1,
            pass: true,
            db: apass,
        };
        let stop = |f0, f1| MaskBand {
            f0,
            f1,
            pass: false,
            db: astop,
        };
        match self.response {
            Response::Lowpass => vec![pass(0., p[0]), stop(s[0], 1.)],
            Response::Highpass => vec![stop(0., s[0]), pass(p[0], 1.)],
            Response::Bandpass => vec![stop(0., s[0]), pass(p[0], p[1]), stop(s[1], 1.)],
            _ => vec![pass(0., p[0]), stop(s[0], s[1]), pass(p[1], 1.)],
        }
    }

    /// Summary line, e.g. "Elliptic bandpass, minimum order".
    pub fn title(&self) -> String {
        if self.method == Method::NotchIir {
            let unit = if self.units == Units::Hz {
                " Hz"
            } else {
                "×π"
            };
            return format!("Notch at {}{unit}, order 2", fmt(self.f0));
        }
        let order = if self.method == Method::NotchIir {
            "order 2".to_owned()
        } else if self.uses_minimum() {
            "minimum order".to_owned()
        } else {
            format!("order {}", self.order)
        };
        format!(
            "{} {}, {order}",
            self.method.label(),
            self.response.label().to_lowercase()
        )
    }

    /// MATLAB script that leaves `b, a` (direct form) or `sos, g` (sections)
    /// in the workspace. Uses only standard Signal Processing Toolbox calls.
    pub fn script(&self) -> Result<String, String> {
        if let Some((_, message)) = self.problems().into_iter().next() {
            return Err(message);
        }
        let mut s = String::new();
        let hz = self.units == Units::Hz;
        let _ = writeln!(s, "% {} (MatRust Filter Designer)", self.title());
        // `w(x)` writes a frequency expression already normalized to 0..1.
        let scale = if hz { "/(Fs/2)" } else { "" };
        if hz {
            let _ = writeln!(s, "Fs = {};  % sample rate (Hz)", fmt(self.fs));
        }
        let unit_note = if hz { "Hz" } else { "x pi rad/sample" };
        let edges = |v: &[f64; 2]| {
            if self.response.two_edges() {
                format!("[{} {}]", fmt(v[0]), fmt(v[1]))
            } else {
                fmt(v[0])
            }
        };
        let rtype = self.response.matlab_type();
        let min = self.uses_minimum();
        let order = self.order;
        let mut iir_zpk = true;
        match self.method {
            Method::Butterworth | Method::Chebyshev1 | Method::Chebyshev2 | Method::Elliptic
                if min =>
            {
                let _ = writeln!(
                    s,
                    "Fpass = {};  Fstop = {};  % {unit_note}",
                    edges(&self.fpass),
                    edges(&self.fstop)
                );
                let _ = writeln!(
                    s,
                    "Apass = {};  Astop = {};  % dB",
                    fmt(self.apass),
                    fmt(self.astop)
                );
                let (ord, call) = match self.method {
                    Method::Butterworth => ("buttord", "butter(n, Wn"),
                    Method::Chebyshev1 => ("cheb1ord", "cheby1(n, Apass, Wn"),
                    Method::Chebyshev2 => ("cheb2ord", "cheby2(n, Astop, Wn"),
                    _ => ("ellipord", "ellip(n, Apass, Astop, Wn"),
                };
                let _ = writeln!(
                    s,
                    "[n, Wn] = {ord}(Fpass{scale}, Fstop{scale}, Apass, Astop);"
                );
                let _ = writeln!(s, "[z, p, k] = {call}, '{rtype}');");
            }
            Method::Butterworth => {
                let _ = writeln!(s, "Fc = {};  % {unit_note}", edges(&self.fc));
                let _ = writeln!(s, "[z, p, k] = butter({order}, Fc{scale}, '{rtype}');");
            }
            Method::Chebyshev1 => {
                let _ = writeln!(s, "Fpass = {};  % {unit_note}", edges(&self.fpass));
                let _ = writeln!(s, "Apass = {};  % dB", fmt(self.apass));
                let _ = writeln!(
                    s,
                    "[z, p, k] = cheby1({order}, Apass, Fpass{scale}, '{rtype}');"
                );
            }
            Method::Chebyshev2 => {
                let _ = writeln!(s, "Fstop = {};  % {unit_note}", edges(&self.fstop));
                let _ = writeln!(s, "Astop = {};  % dB", fmt(self.astop));
                let _ = writeln!(
                    s,
                    "[z, p, k] = cheby2({order}, Astop, Fstop{scale}, '{rtype}');"
                );
            }
            Method::Elliptic => {
                let _ = writeln!(s, "Fpass = {};  % {unit_note}", edges(&self.fpass));
                let _ = writeln!(
                    s,
                    "Apass = {};  Astop = {};  % dB",
                    fmt(self.apass),
                    fmt(self.astop)
                );
                let _ = writeln!(
                    s,
                    "[z, p, k] = ellip({order}, Apass, Astop, Fpass{scale}, '{rtype}');"
                );
            }
            Method::Bessel => {
                // Analog prototype, then the bilinear transform matched at Fc.
                if !hz {
                    let _ = writeln!(s, "Fs = 2;  % normalized: Nyquist = 1");
                }
                let _ = writeln!(s, "Fc = {};  % {unit_note}", fmt(self.fc[0]));
                let _ = writeln!(s, "[z, p, k] = besself({order}, 2*pi*Fc);");
                let _ = writeln!(s, "[z, p, k] = bilinear(z, p, k, Fs, Fc);");
            }
            Method::NotchIir => {
                iir_zpk = false;
                let _ = writeln!(
                    s,
                    "F0 = {};  BW = {};  % {unit_note}",
                    fmt(self.f0),
                    fmt(self.bandwidth)
                );
                let _ = writeln!(s, "[b, a] = iirnotch(F0{scale}, BW{scale});");
            }
            Method::Window if min => {
                let (f, a, dev) = self.kaiser_bands();
                let _ = writeln!(
                    s,
                    "Apass = {};  Astop = {};  % dB",
                    fmt(self.apass),
                    fmt(self.astop)
                );
                let _ = writeln!(
                    s,
                    "dpass = (10^(Apass/20) - 1)/(10^(Apass/20) + 1);  dstop = 10^(-Astop/20);"
                );
                let fs_arg = if hz { ", Fs" } else { "" };
                let _ = writeln!(
                    s,
                    "[n, Wn, beta, ftype] = kaiserord({f}, {a}, {dev}{fs_arg});  % {unit_note}"
                );
                if matches!(self.response, Response::Highpass | Response::Bandstop) {
                    // fir1 needs an even order when the gain at Nyquist is 1.
                    s.push_str("n = n + rem(n, 2);  % even order: gain 1 at Nyquist\n");
                }
                let _ = writeln!(s, "b = fir1(n, Wn, ftype, kaiser(n + 1, beta), 'noscale');");
            }
            Method::Window => {
                let _ = writeln!(s, "Fc = {};  % {unit_note}", edges(&self.fc));
                let (_, name) = WINDOWS[self.window];
                let win = if self.window == KAISER {
                    format!("kaiser({}, {})", order + 1, fmt(self.beta))
                } else {
                    format!("{name}({})", order + 1)
                };
                let _ = writeln!(s, "b = fir1({order}, Fc{scale}, '{rtype}', {win});");
            }
            Method::Equiripple if min => {
                let (f, a, dev) = self.kaiser_bands();
                let _ = writeln!(
                    s,
                    "Apass = {};  Astop = {};  % dB",
                    fmt(self.apass),
                    fmt(self.astop)
                );
                let _ = writeln!(
                    s,
                    "dpass = (10^(Apass/20) - 1)/(10^(Apass/20) + 1);  dstop = 10^(-Astop/20);"
                );
                let fs_arg = if hz { ", Fs" } else { "" };
                let _ = writeln!(
                    s,
                    "[n, fo, ao, w] = firpmord({f}, {a}, {dev}{fs_arg});  % {unit_note}"
                );
                if matches!(self.response, Response::Highpass | Response::Bandstop) {
                    let _ = writeln!(
                        s,
                        "n = n + rem(n, 2);  % highpass/bandstop need an even order"
                    );
                }
                let _ = writeln!(s, "b = firpm(n, fo, ao, w);");
            }
            Method::Equiripple | Method::LeastSquares | Method::FrequencySampling => {
                let call = match self.method {
                    Method::Equiripple => "firpm",
                    Method::LeastSquares => "firls",
                    _ => "fir2",
                };
                let weighted = call != "fir2";
                match self.response {
                    Response::Differentiator => {
                        let _ = writeln!(
                            s,
                            "Fpass = {};  % {unit_note}, end of the differentiating band",
                            fmt(self.fpass[0])
                        );
                        let _ = writeln!(
                            s,
                            "b = {call}({order}, [0 Fpass{scale}], [0 pi*Fpass{scale}], 'differentiator');"
                        );
                    }
                    Response::Hilbert => {
                        let _ = writeln!(s, "Fpass = {};  % {unit_note}", edges(&self.fpass));
                        let _ = writeln!(s, "b = {call}({order}, Fpass{scale}, [1 1], 'hilbert');");
                    }
                    Response::Multiband | Response::Arbitrary => {
                        let freqs = parse_vector(&self.freqs)?;
                        let amps = parse_vector(&self.amps)?;
                        let _ = writeln!(s, "F = [{}];  % {unit_note}", format_vector(&freqs));
                        let _ = writeln!(s, "A = [{}];", format_vector(&amps));
                        let weights = parse_vector(&self.weights).unwrap_or_default();
                        if weighted && !weights.is_empty() {
                            let _ = writeln!(s, "W = [{}];", format_vector(&weights));
                            let _ = writeln!(s, "b = {call}({order}, F{scale}, A, W);");
                        } else {
                            let _ = writeln!(s, "b = {call}({order}, F{scale}, A);");
                        }
                    }
                    _ => {
                        let (f, a, w) = self.band_vectors();
                        let _ = writeln!(
                            s,
                            "Fpass = {};  Fstop = {};  % {unit_note}",
                            edges(&self.fpass),
                            edges(&self.fstop)
                        );
                        let nyq = if hz { "Fs/2" } else { "1" };
                        let f = f.replace("NYQ", nyq);
                        let _ = writeln!(s, "F = {f}{scale};");
                        let _ = writeln!(s, "A = {a};");
                        if weighted {
                            let _ = writeln!(
                                s,
                                "Wpass = {};  Wstop = {};",
                                fmt(self.wpass),
                                fmt(self.wstop)
                            );
                            let _ = writeln!(s, "b = {call}({order}, F, A, {w});");
                        } else {
                            let _ = writeln!(s, "b = {call}({order}, F, A);");
                        }
                    }
                }
            }
        }
        if self.method.iir() {
            match (self.structure, iir_zpk) {
                (Structure::Sos, true) => s.push_str("[sos, g] = zp2sos(z, p, k);\n"),
                (Structure::Sos, false) => s.push_str("[sos, g] = tf2sos(b, a);\n"),
                (Structure::Direct, true) => s.push_str("[b, a] = zp2tf(z, p, k);\n"),
                (Structure::Direct, false) => {}
            }
        } else {
            s.push_str("a = 1;\n");
        }
        Ok(s)
    }
    /// The design script followed by filtering workspace signal `input`
    /// into `y` (`filtfilt` for zero phase), and a plot of the result.
    pub fn apply_script(&self, script: &str, input: &str, zero_phase: bool) -> String {
        // The input is read first: the design script may reuse its name (b, sos, ...).
        let call = match (self.outputs()[0], zero_phase) {
            ("sos", false) => "y = g*sosfilt(sos, x);",
            ("sos", true) => "y = filtfilt(sos, g, x);",
            (_, false) => "y = filter(b, a, x);",
            (_, true) => "y = filtfilt(b, a, x);",
        };
        let name = if zero_phase { "filtfilt" } else { "filter" };
        format!("x = {input};\n{script}{call}\nplot(y); title('{input} after {name}');\n")
    }
    /// A `designfilt` call building the same filter as a digitalFilter `d`,
    /// when designfilt has that specification set (a set order with Window
    /// (Hamming), Butterworth or Chebyshev II on a classic response).
    pub fn designfilt(&self) -> Option<String> {
        if !self.response.classic() || self.uses_minimum() || !self.problems().is_empty() {
            return None;
        }
        let two = self.response.two_edges();
        let kind = match self.response {
            Response::Lowpass => "lowpass",
            Response::Highpass => "highpass",
            Response::Bandpass => "bandpass",
            _ => "bandstop",
        };
        let pairs = |name: &str, v: &[f64; 2]| {
            if two {
                format!("'{name}1', {}, '{name}2', {}", fmt(v[0]), fmt(v[1]))
            } else {
                format!("'{name}', {}", fmt(v[0]))
            }
        };
        let order = if two { 2 * self.order } else { self.order };
        let (suffix, specs) = match self.method {
            Method::Window if self.window == 0 => ("fir", pairs("CutoffFrequency", &self.fc)),
            Method::Butterworth => ("iir", pairs("HalfPowerFrequency", &self.fc)),
            Method::Chebyshev2 => (
                "iir",
                format!(
                    "{}, 'StopbandAttenuation', {}",
                    pairs("StopbandFrequency", &self.fstop),
                    fmt(self.astop)
                ),
            ),
            _ => return None,
        };
        let fs = if self.units == Units::Hz {
            format!(", 'SampleRate', {}", fmt(self.fs))
        } else {
            String::new()
        };
        let order = if suffix == "fir" { self.order } else { order };
        Some(format!(
            "d = designfilt('{kind}{suffix}', 'FilterOrder', {order}, {specs}{fs})\n"
        ))
    }
    /// The output variables the script leaves.
    pub fn outputs(&self) -> [&'static str; 2] {
        if self.method.iir() && self.structure == Structure::Sos {
            ["sos", "g"]
        } else {
            ["b", "a"]
        }
    }
    /// Edge/gain/deviation vectors for kaiserord and firpmord, in script form.
    fn kaiser_bands(&self) -> (String, String, String) {
        let (p, s) = (self.fpass, self.fstop);
        let v = |x: &[f64]| format!("[{}]", format_vector(x));
        match self.response {
            Response::Lowpass => (v(&[p[0], s[0]]), "[1 0]".into(), "[dpass dstop]".into()),
            Response::Highpass => (v(&[s[0], p[0]]), "[0 1]".into(), "[dstop dpass]".into()),
            Response::Bandpass => (
                v(&[s[0], p[0], p[1], s[1]]),
                "[0 1 0]".into(),
                "[dstop dpass dstop]".into(),
            ),
            _ => (
                v(&[p[0], s[0], s[1], p[1]]),
                "[1 0 1]".into(),
                "[dpass dstop dpass]".into(),
            ),
        }
    }
    /// firpm/firls/fir2 band vectors over 0..Nyquist ("NYQ" placeholder).
    fn band_vectors(&self) -> (String, String, String) {
        match self.response {
            Response::Lowpass => (
                "[0 Fpass Fstop NYQ]".into(),
                "[1 1 0 0]".into(),
                "[Wpass Wstop]".into(),
            ),
            Response::Highpass => (
                "[0 Fstop Fpass NYQ]".into(),
                "[0 0 1 1]".into(),
                "[Wstop Wpass]".into(),
            ),
            Response::Bandpass => (
                "[0 Fstop(1) Fpass(1) Fpass(2) Fstop(2) NYQ]".into(),
                "[0 0 1 1 0 0]".into(),
                "[Wstop Wpass Wstop]".into(),
            ),
            _ => (
                "[0 Fpass(1) Fstop(1) Fstop(2) Fpass(2) NYQ]".into(),
                "[1 1 0 0 1 1]".into(),
                "[Wpass Wstop Wpass]".into(),
            ),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaskBand {
    pub f0: f64,
    pub f1: f64,
    pub pass: bool,
    /// Apass for a passband, Astop for a stopband, when the method uses it.
    pub db: Option<f64>,
}

/// Shortest text that reads back as the same number.
pub fn fmt(x: f64) -> String {
    if x == x.trunc() && x.abs() < 1e15 {
        format!("{}", x as i64)
    } else {
        format!("{x}")
    }
}
pub fn format_vector(values: &[f64]) -> String {
    values.iter().map(|v| fmt(*v)).collect::<Vec<_>>().join(" ")
}
/// "0 0.2, 0.3 [1]" → numbers; brackets, commas and spaces separate.
pub fn parse_vector(text: &str) -> Result<Vec<f64>, String> {
    text.split(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | '[' | ']'))
        .filter(|t| !t.is_empty())
        .map(|t| {
            t.parse::<f64>()
                .map_err(|_| format!("'{t}' is not a number"))
        })
        .collect()
}

/// Designed coefficients, as the script left them.
#[derive(Clone, Debug, PartialEq)]
pub enum Coefficients {
    Tf { b: Vec<f64>, a: Vec<f64> },
    Sos { sos: Vec<[f64; 6]>, g: f64 },
}
impl Coefficients {
    /// Complex response at normalized frequencies (1 = Nyquist).
    pub fn response(
        &self,
        f: &[f64],
        cancel: &std::sync::atomic::AtomicBool,
    ) -> matrust::Result<Vec<[f64; 2]>> {
        match self {
            Self::Tf { b, a } => matrust::dsp::response(b, a, f, cancel),
            Self::Sos { sos, g } => {
                let m = sos_matrix(sos)?;
                let h = matrust::dsp::sos_response(&m, f, cancel)?;
                Ok(h.into_iter().map(|[r, i]| [r * g, i * g]).collect())
            }
        }
    }
    pub fn order(&self) -> usize {
        match self {
            Self::Tf { b, a } => {
                let trim = |v: &[f64]| v.iter().rposition(|x| *x != 0.).unwrap_or(0);
                trim(b).max(trim(a))
            }
            Self::Sos { sos, .. } => sos
                .iter()
                .map(|s| if s[2] != 0. || s[5] != 0. { 2 } else { 1 })
                .sum(),
        }
    }
}
pub fn sos_matrix(sos: &[[f64; 6]]) -> matrust::Result<matrust::Matrix> {
    let rows = sos.len();
    let data = (0..6).flat_map(|c| sos.iter().map(move |r| r[c])).collect();
    matrust::Matrix::from_column_vec(rows, 6, data)
}

/// Run the spec's script in `engine` and read the coefficients back.
pub fn design(spec: &Spec, engine: &mut matrust::Engine) -> matrust::Result<Coefficients> {
    engine.execute(&spec.script()?)?;
    let get = |name: &str| {
        engine
            .variables
            .get(name)
            .cloned()
            .ok_or_else(|| format!("The design script did not set '{name}'"))
    };
    if spec.outputs()[0] == "sos" {
        let (sos, g) = (get("sos")?, get("g")?);
        if sos.cols() != 6 || sos.rows() == 0 {
            return Err("The design returned no second-order sections".into());
        }
        let rows = sos.rows();
        let sections = (0..rows)
            .map(|r| std::array::from_fn(|c| sos.data()[c * rows + r]))
            .collect();
        Ok(Coefficients::Sos {
            sos: sections,
            g: g.as_scalar()?,
        })
    } else {
        let (b, a) = (get("b")?, get("a")?);
        if b.data().iter().chain(a.data()).any(|v| !v.is_finite()) {
            return Err("The design produced non-finite coefficients".into());
        }
        Ok(Coefficients::Tf {
            b: b.data().to_vec(),
            a: a.data().to_vec(),
        })
    }
}

#[cfg(test)]
#[path = "filter_spec_tests.rs"]
mod tests;
