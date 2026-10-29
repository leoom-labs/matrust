//! Single-rate Forward Euler with internal scalar initial condition.
use serde::{Deserialize, Serialize};

/// Other integration/accumulation methods are intentionally not deserializable.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum IntegratorMethod {
    #[default]
    ForwardEuler,
}

pub(super) fn validate(initial: f64, gain: f64) -> crate::Result<()> {
    if !initial.is_finite() || !gain.is_finite() || gain <= 0. {
        return Err(
            "Forward Euler requires a finite initial output and positive finite gain".into(),
        );
    }
    Ok(())
}
