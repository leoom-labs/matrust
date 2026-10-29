use super::{Engine, Value};
use crate::{
    Array, Result,
    dsp::{self, FftOptions, TransformDirection},
};

fn integer(value: &Value) -> Result<usize> {
    let n = match value {
        Value::Matrix(a) if a.len() == 1 => a.data()[0],
        Value::Logical(a) if a.len() == 1 => f64::from(a.data()[0]),
        _ => return Err("FFT size/dimension must be a real or logical scalar".into()),
    };
    if !n.is_finite() || n < 0. || n.fract() != 0. || n > crate::matrix::MAX_ELEMENTS as f64 {
        return Err("FFT size/dimension must be a bounded nonnegative integer".into());
    }
    Ok(n as usize)
}

impl Engine {
    pub(super) fn fourier_call(&self, name: &str, args: &[Value]) -> Result<Value> {
        self.fourier_with_retained(name, args, 0)
    }

    /// Internal transforms also account for the caller's live temporary buffers.
    pub(super) fn fourier_with_retained(
        &self,
        name: &str,
        args: &[Value],
        retained_elements: usize,
    ) -> Result<Value> {
        self.check_cancel()?;
        if !(1..=3).contains(&args.len()) {
            return Err("fft/ifft accept X, optional n (or []), and dimension 1 or 2; symmetry flags are not implemented".into());
        }
        let input = match &args[0] {
            Value::Matrix(a) => Array::Numeric(a.clone()),
            Value::Logical(a) => Array::Logical(a.clone()),
            Value::Complex(a) => Array::Complex(a.clone()),
            _ => return Err("fft/ifft require a real, logical or complex array".into()),
        };
        let length = match args.get(1) {
            None => None,
            Some(Value::Matrix(a)) if a.is_empty() => None,
            Some(value) => Some(integer(value)?),
        };
        let dimension = args.get(2).map(integer).transpose()?;
        let remaining = (32usize * 1024 * 1024)
            .saturating_sub(self.evaluated_elements)
            .saturating_sub(retained_elements)
            .checked_mul(8)
            .ok_or("FFT evaluation budget overflow")?;
        let direction = if name == "ifft" {
            TransformDirection::Inverse
        } else {
            TransformDirection::Forward
        };
        match dsp::fourier(
            &input,
            FftOptions { length, dimension },
            direction,
            remaining,
            &self.cancel,
        )? {
            Array::Numeric(a) => Ok(Value::Matrix(a)),
            Array::Complex(a) => Ok(Value::Complex(a)),
            _ => unreachable!("Fourier results are double or complex double"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn statement_budget_precedes_fft_planning_and_output_allocation() {
        let engine = Engine {
            evaluated_elements: 32 * 1024 * 1024,
            ..Engine::default()
        };
        let args = [Value::Matrix(crate::Matrix::scalar(1.))];
        assert!(
            engine
                .fourier_call("fft", &args)
                .unwrap_err()
                .contains("budget")
        );
    }
    #[test]
    fn fft_rejects_shared_character_arguments_before_generic_text_conversion() {
        let mut engine = Engine::default();
        let row = crate::CharacterRow::from_units(vec![0xd800; 65536], &engine.cancel).unwrap();
        let character = Value::Container(Array::Character(row.clone()));
        let mut out = super::super::Execution::default();
        let error = engine
            .call("fft", vec![character.clone(); 128], &mut out)
            .unwrap_err();
        assert!(error.contains("fft/ifft accept X"), "{error}");
        let error = engine.call("ifft", vec![character], &mut out).unwrap_err();
        assert!(
            error.contains("require a real, logical or complex"),
            "{error}"
        );
        assert_eq!(row.units()[0], 0xd800);
    }
}
