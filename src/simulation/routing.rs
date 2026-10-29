//! Finite real scalar arithmetic/routing in the fixed-rate simulator.
use crate::Result;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum ProductInput {
    #[serde(rename = "*")]
    Multiply,
    #[serde(rename = "/")]
    Divide,
}
impl ProductInput {
    pub fn label(self) -> &'static str {
        match self {
            Self::Multiply => "Multiply (*)",
            Self::Divide => "Divide (/)",
        }
    }
}
pub(super) fn default_product_inputs() -> Vec<ProductInput> {
    vec![ProductInput::Multiply; 2]
}
pub(super) fn validate_product(operations: &[ProductInput]) -> Result<()> {
    if !(1..=32).contains(&operations.len()) {
        return Err("Product requires 1..32 scalar input operations".into());
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum SwitchCriteria {
    #[default]
    #[serde(rename = "u2 > Threshold")]
    Greater,
    #[serde(rename = "u2 >= Threshold")]
    GreaterEqual,
    #[serde(rename = "u2 ~= 0")]
    Nonzero,
}
impl SwitchCriteria {
    pub fn label(self) -> &'static str {
        match self {
            Self::Greater => "u2 > Threshold",
            Self::GreaterEqual => "u2 >= Threshold",
            Self::Nonzero => "u2 ~= 0",
        }
    }
    pub(super) fn selects_first(self, control: f64, threshold: f64) -> bool {
        match self {
            Self::Greater => control > threshold,
            Self::GreaterEqual => control >= threshold,
            Self::Nonzero => control != 0.,
        }
    }
}

/// Port order is explicit; overflowing intermediates cannot become hidden zeros.
/// Validation establishes equal nonempty lengths and valid source indices.
pub(super) fn product(operations: &[ProductInput], inputs: &[usize], values: &[f64]) -> f64 {
    let first = values[inputs[0]];
    let mut value = match operations[0] {
        ProductInput::Multiply => first,
        ProductInput::Divide => 1. / first,
    };
    for (operation, &source) in operations[1..].iter().zip(&inputs[1..]) {
        if !value.is_finite() {
            return value;
        }
        value = match operation {
            ProductInput::Multiply => value * values[source],
            ProductInput::Divide => value / values[source],
        };
    }
    value
}

#[cfg(test)]
std::thread_local! {
    static STOP_AT: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
    static LAST_CHECKPOINT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
#[cfg(test)]
pub(super) fn cancellation_probe(sample: usize, cancel: &std::sync::atomic::AtomicBool) {
    if let Some(stop) = STOP_AT.get() {
        LAST_CHECKPOINT.set(sample);
        if sample == stop {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::{Block, Model, Node};
    use std::sync::atomic::{AtomicBool, Ordering};
    #[test]
    fn scalar_routing_active_stop_after_completed_samples_keeps_model_retryable() {
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                STOP_AT.set(None);
            }
        }
        let model = Model {
            steps: 1024,
            nodes: vec![
                Node {
                    name: "One".into(),
                    block: Block::Constant { value: 1. },
                    inputs: vec![],
                },
                Node {
                    name: "Product".into(),
                    block: Block::Product {
                        operations: vec![ProductInput::Multiply; 32],
                    },
                    inputs: vec![0; 32],
                },
                Node {
                    name: "Minus".into(),
                    block: Block::UnaryMinus,
                    inputs: vec![1],
                },
                Node {
                    name: "Switch".into(),
                    block: Block::Switch {
                        criteria: SwitchCriteria::Greater,
                        threshold: 0.,
                    },
                    inputs: vec![2, 0, 1],
                },
                Node {
                    name: "Output".into(),
                    block: Block::Scope,
                    inputs: vec![3],
                },
            ],
            ..Model::default()
        };
        let original = model.clone();
        let cancel = AtomicBool::new(false);
        let reference = model.run(&cancel).unwrap();
        let reset = Reset;
        STOP_AT.set(Some(256));
        assert!(model.run(&cancel).is_err());
        assert!(cancel.load(Ordering::Relaxed));
        assert_eq!(LAST_CHECKPOINT.get(), 256);
        assert!(model.same_storage(&original));
        drop(reset);
        cancel.store(false, Ordering::Relaxed);
        let retry = model.run(&cancel).unwrap();
        assert_eq!(
            retry.traces[0].values.data(),
            reference.traces[0].values.data()
        );
    }
}

#[cfg(test)]
mod integrator_cancellation_tests {
    use super::*;
    use crate::simulation::{Block, IntegratorMethod, Model, Node};
    use std::sync::atomic::{AtomicBool, Ordering};
    #[test]
    fn active_integrator_stop_discards_partial_trace_and_retry_restarts_from_ic() {
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                STOP_AT.set(None);
            }
        }
        let model = Model {
            sample_time: 0.25,
            steps: 1024,
            nodes: vec![
                Node {
                    name: "Input".into(),
                    block: Block::Constant { value: 1. },
                    inputs: vec![],
                },
                Node {
                    name: "Integrator".into(),
                    block: Block::DiscreteIntegrator {
                        initial: 2.,
                        gain: 1.,
                        method: IntegratorMethod::ForwardEuler,
                    },
                    inputs: vec![0],
                },
                Node {
                    name: "Scope".into(),
                    block: Block::Scope,
                    inputs: vec![1],
                },
            ],
            ..Model::default()
        };
        let before = model.clone();
        let cancel = AtomicBool::new(false);
        let reset = Reset;
        STOP_AT.set(Some(256));
        assert!(model.run(&cancel).is_err());
        assert!(cancel.load(Ordering::Relaxed));
        assert_eq!(LAST_CHECKPOINT.get(), 256);
        assert!(model.same_storage(&before));
        drop(reset);
        cancel.store(false, Ordering::Relaxed);
        let result = model.run(&cancel).unwrap();
        for (n, v) in result.traces[0].values.data().iter().enumerate() {
            assert_eq!(*v, 2. + n as f64 * 0.25);
        }
        assert!(model.run(&AtomicBool::new(true)).is_err());
        assert!(model.same_storage(&before));
    }
}
