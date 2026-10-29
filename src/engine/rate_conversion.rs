use super::{Engine, Value};
use crate::{Matrix, Result, dsp, matrix::MAX_ELEMENTS};

impl Engine {
    pub(super) fn rate_conversion_call(&self, name: &str, args: &[Value]) -> Result<Value> {
        self.check_cancel()?;
        let numeric = |index: usize| -> Result<&Matrix> {
            match args.get(index) {
                Some(Value::Matrix(value)) => Ok(value),
                _ => Err(format!(
                    "{name} requires real numeric arrays; logical and complex inputs are not implemented"
                )),
            }
        };
        let integer = |index: usize, allow_zero: bool| -> Result<usize> {
            let value = numeric(index)?.as_scalar()?;
            let minimum = if allow_zero { 0. } else { 1. };
            if !value.is_finite()
                || value.fract() != 0.
                || value < minimum
                || value > MAX_ELEMENTS as f64
            {
                return Err(format!(
                    "{name} factors/phase must be finite bounded integers (maximum {MAX_ELEMENTS})"
                ));
            }
            Ok(value as usize)
        };
        let (rows, cols) = match name {
            "upsample" | "downsample" => {
                if !(2..=3).contains(&args.len()) {
                    return Err(format!(
                        "Use {name}(x,n[,phase]); phase must be from 0 to n-1"
                    ));
                }
                let n = integer(1, false)?;
                let phase = if args.len() == 3 {
                    integer(2, true)?
                } else {
                    0
                };
                dsp::rate_output_shape(numeric(0)?, n, phase, name == "upsample")?
            }
            "upfirdn" => {
                if !(2..=4).contains(&args.len()) {
                    return Err("Use upfirdn(x,h[,p[,q]]); FIR filter must be supplied".into());
                }
                let p = if args.len() >= 3 {
                    integer(2, false)?
                } else {
                    1
                };
                let q = if args.len() == 4 {
                    integer(3, false)?
                } else {
                    1
                };
                dsp::upfirdn_output_shape(numeric(0)?, numeric(1)?, p, q)?
            }
            _ => return Err("Unknown rate-conversion function".into()),
        };
        let count = Matrix::checked_len(rows, cols)?;
        if self
            .evaluated_elements
            .checked_add(count)
            .is_none_or(|n| n > 32 * 1024 * 1024)
        {
            return Err("Statement exceeds its 256 MiB evaluation budget; split the calculation into smaller steps".into());
        }
        let value = match name {
            "upsample" | "downsample" => {
                let n = integer(1, false)?;
                let phase = if args.len() == 3 {
                    integer(2, true)?
                } else {
                    0
                };
                if name == "upsample" {
                    dsp::upsample(numeric(0)?, n, phase, &self.cancel)?
                } else {
                    dsp::downsample(numeric(0)?, n, phase, &self.cancel)?
                }
            }
            _ => {
                let p = if args.len() >= 3 {
                    integer(2, false)?
                } else {
                    1
                };
                let q = if args.len() == 4 {
                    integer(3, false)?
                } else {
                    1
                };
                dsp::upfirdn(numeric(0)?, numeric(1)?, p, q, &self.cancel)?
            }
        };
        Ok(Value::Matrix(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rate_results_are_preflighted_before_expanding_output() {
        let engine = Engine {
            evaluated_elements: 32 * 1024 * 1024 - 1,
            ..Engine::default()
        };
        let x = Matrix::scalar(2.);
        let args = [Value::Matrix(x.clone()), Value::Matrix(Matrix::scalar(2.))];
        assert!(
            engine
                .rate_conversion_call("upsample", &args)
                .unwrap_err()
                .contains("evaluation budget")
        );
        assert_eq!(x.data(), &[2.]);
        assert_eq!(engine.evaluated_elements, 32 * 1024 * 1024 - 1);
    }
}
