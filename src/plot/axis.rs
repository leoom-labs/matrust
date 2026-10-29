use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum AxisScale {
    #[default]
    Linear,
    Log,
}
impl AxisScale {
    pub fn validate_limits(self, range: [f64; 2]) -> Result<()> {
        super::validate_limits(range)?;
        if self == Self::Log && !(range[0] > 0. || range[1] < 0.) {
            return Err("Logarithmic limits must have the same nonzero sign".into());
        }
        if self == Self::Log && range[0].abs().log10() == range[1].abs().log10() {
            return Err("Logarithmic limits are too close to represent".into());
        }
        Ok(())
    }
}

/// Monotonic display transform. Negative-only logarithmic axes retain data order.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AxisTransform {
    pub scale: AxisScale,
    pub negative: bool,
}
impl AxisTransform {
    pub fn accepts(self, value: f64) -> bool {
        value.is_finite()
            && (self.scale == AxisScale::Linear
                || if self.negative {
                    value < 0.
                } else {
                    value > 0.
                })
    }
    pub fn forward(self, value: f64) -> f64 {
        if !self.accepts(value) {
            return f64::NAN;
        }
        match (self.scale, self.negative) {
            (AxisScale::Linear, _) => value,
            (_, false) => value.log10(),
            (_, true) => -(-value).log10(),
        }
    }
    pub fn inverse(self, value: f64) -> f64 {
        if self.scale == AxisScale::Linear {
            return value;
        }
        let exponent = if self.negative { -value } else { value };
        let result = 10_f64
            .powf(exponent.clamp(f64::from_bits(1).log10(), f64::MAX.log10()))
            .clamp(f64::from_bits(1), f64::MAX);
        if self.negative { -result } else { result }
    }
    pub fn label(self, display: f64) -> String {
        let value = self.inverse(display);
        if value.abs() >= 1e5 || (value != 0. && value.abs() < 1e-3) {
            if self.scale == AxisScale::Log && (display - display.round()).abs() < 1e-10 {
                format!(
                    "{}1e{:.0}",
                    if self.negative { "-" } else { "" },
                    if self.negative { -display } else { display }
                )
            } else {
                format!("{value:.3e}")
            }
        } else {
            format!("{value:.5}")
                .trim_end_matches('0')
                .trim_end_matches('.')
                .to_owned()
        }
    }
    /// Bounded logarithmic ticks in display coordinates, including narrow zooms.
    pub fn ticks(self, range: [f64; 2]) -> Vec<f64> {
        if !range.iter().all(|x| x.is_finite()) || range[0] >= range[1] {
            return Vec::new();
        }
        let span = range[1] - range[0];
        if !span.is_finite() {
            return Vec::new();
        }
        if span < 1. {
            return (0..=5).map(|i| range[0] + span * i as f64 / 5.).collect();
        }
        let step = (span / 8.).ceil().max(1.);
        let first = (range[0] / step).ceil() * step;
        (0..=9)
            .map(|i| first + i as f64 * step)
            .filter(|x| *x <= range[1])
            .collect()
    }
}
