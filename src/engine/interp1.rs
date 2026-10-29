//! Typed interpolation admission before generic text/graphics projection.
use super::{Engine, Value};
use crate::{
    Array, Result,
    array::{InterpolationMethod, interpolate_vectors},
};

impl Engine {
    pub(super) fn interp1_call(&self, args: &[Value]) -> Result<Value> {
        self.check_cancel()?;
        if !(3..=5).contains(&args.len()) {
            return Err("interp1 requires explicit x,v,xq, optional linear/nearest method and scalar fill; implicit coordinates are unsupported".into());
        }
        let (Value::Matrix(x), Value::Matrix(v), Value::Matrix(q)) = (&args[0], &args[1], &args[2])
        else {
            return Err("interp1 requires real double coordinates, samples and queries; logical/complex/text/cell/graphics inputs are unsupported".into());
        };
        let decoded;
        let method = match args.get(3) {
            None => InterpolationMethod::Linear,
            Some(value) => {
                let text = match value {
                    Value::Text(text) => text.as_str(),
                    Value::Container(Array::Character(text)) if text.rows()==1 && matches!(text.cols(),6|7) => {
                        decoded = self.text_value(value.clone())?;
                        decoded.as_str()
                    }
                    _ => return Err("interp1 method must be 'linear' or 'nearest'; string arrays and other options are unsupported".into()),
                };
                match text {
                    "linear" => InterpolationMethod::Linear,
                    "nearest" => InterpolationMethod::Nearest,
                    _ => return Err("interp1 supports only linear/nearest with scalar fill; extrapolation and other methods are unsupported".into()),
                }
            }
        };
        let fill = match args.get(4) {
            None => f64::NAN,
            Some(Value::Matrix(fill)) => fill.as_scalar()?,
            _ => return Err("interp1 outside-domain fill must be a real double scalar; algorithmic extrapolation is unsupported".into()),
        };
        if self
            .evaluated_elements
            .checked_add(q.len())
            .is_none_or(|n| n > 32 * 1024 * 1024)
        {
            return Err("Statement exceeds its 256 MiB evaluation budget; split the calculation into smaller steps".into());
        }
        Ok(Value::Matrix(interpolate_vectors(
            x,
            v,
            q,
            method,
            fill,
            &self.cancel,
        )?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Matrix;
    #[test]
    fn output_budget_preflight_borrows_inputs() {
        let x = Matrix::from_column_slice(1, 2, &[0., 1.]).unwrap();
        let q = Matrix::from_column_slice(1, 3, &[0., 0.5, 1.]).unwrap();
        let arguments = [
            Value::Matrix(x.clone()),
            Value::Matrix(x.clone()),
            Value::Matrix(q.clone()),
        ];
        let engine = Engine {
            evaluated_elements: 32 * 1024 * 1024 - 2,
            ..Engine::default()
        };
        assert!(
            engine
                .interp1_call(&arguments)
                .unwrap_err()
                .contains("evaluation budget")
        );
        let Value::Matrix(shared) = &arguments[0] else {
            panic!("real input");
        };
        assert_eq!(shared.data().as_ptr(), x.data().as_ptr());
        let engine = Engine {
            evaluated_elements: 32 * 1024 * 1024 - 3,
            ..Engine::default()
        };
        let Value::Matrix(result) = engine.interp1_call(&arguments).unwrap() else {
            panic!("real result");
        };
        assert_eq!(result.data(), q.data());
    }
}
