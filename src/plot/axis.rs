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
    pub fn validate_axis_limits(self, range: [f64; 2]) -> Result<()> {
        super::validate_axis_limits(range)?;
        if range[0].is_finite() && range[1].is_finite() {
            return self.validate_limits(range);
        }
        if self == Self::Log {
            if range[0].is_finite() && range[0] <= 0. && range[1] == f64::INFINITY {
                return Err("Logarithmic limits must have the same nonzero sign".into());
            }
            if range[1].is_finite() && range[1] >= 0. && range[0] == f64::NEG_INFINITY {
                return Err("Logarithmic limits must have the same nonzero sign".into());
            }
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

/// Most minor grid lines one axis draws; a denser request draws none.
pub const MAX_MINOR_TICKS: usize = 512;
/// MATLAB's default `MinorGridAlpha` (the major `GridAlpha` default is 0.15).
pub const MINOR_GRID_ALPHA: f64 = 0.25;

/// Minor tick positions in display coordinates, major positions excluded.
/// Octave's rule (`calc_ticks_and_lims`): four minor ticks per linear major
/// interval, i.e. every `major_step / 5`, continued to both axis ends; on a
/// logarithmic axis 2..9 x each decade, or the skipped integer decades when
/// the major step spans several decades. `major_step` is the linear major
/// spacing; logarithmic axes take their decade step from
/// [`AxisTransform::ticks`] and ignore it.
pub fn minor_ticks(axis: AxisTransform, display: [f64; 2], major_step: f64) -> Vec<f64> {
    let [low, high] = display;
    if !(low.is_finite() && high.is_finite() && low < high) {
        return Vec::new();
    }
    let multiples = |step: f64, every: f64| -> Vec<f64> {
        // Indices i * step inside the range; i a multiple of `every` is major.
        // The slack keeps an end position that rounding puts just outside.
        let first = (low / step - 1e-9).ceil();
        let last = (high / step + 1e-9).floor();
        if !(first.is_finite() && last.is_finite()) || last - first > MAX_MINOR_TICKS as f64 {
            return Vec::new();
        }
        (0..=(last - first).max(-1.) as i64)
            .map(|k| first + k as f64)
            .filter(|i| i.rem_euclid(every) != 0.)
            .map(|i| i * step)
            .collect()
    };
    if axis.scale == AxisScale::Linear {
        let step = major_step / 5.;
        return if step.is_finite() && step > 0. {
            multiples(step, 5.)
        } else {
            Vec::new()
        };
    }
    let span = high - low;
    let decades = if span < 1. {
        1.
    } else {
        (span / 8.).ceil().max(1.)
    };
    if decades > 1. {
        return multiples(1., decades);
    }
    // Display is +-log10|v|; walk magnitudes upward and mirror back.
    let (a, b) = if axis.negative {
        (-high, -low)
    } else {
        (low, high)
    };
    let mut values: Vec<f64> = (a.floor() as i64..=b.floor() as i64)
        .flat_map(|e| (2..=9).map(move |k| e as f64 + f64::from(k).log10()))
        .filter(|m| (a..=b).contains(m))
        .map(|m| if axis.negative { -m } else { m })
        .collect();
    values.sort_by(f64::total_cmp);
    values
}
