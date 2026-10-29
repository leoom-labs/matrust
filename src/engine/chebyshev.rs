use super::{Engine, Value};
use crate::{
    Array, ComplexMatrix, Matrix, Result,
    dsp::{self, Band},
};

fn scalar(value: &Value) -> Result<f64> {
    match value {
        Value::Matrix(a) if a.len() == 1 => Ok(a.data()[0]),
        _ => Err("cheby1 order and ripple require real double scalars".into()),
    }
}
fn output(a: ComplexMatrix) -> Result<Value> {
    if a.data().iter().all(|value| value.im == 0.) {
        Ok(Value::Matrix(Matrix::from_column_vec(
            a.rows(),
            a.cols(),
            a.data().iter().map(|v| v.re).collect(),
        )?))
    } else {
        Ok(Value::Complex(a))
    }
}
impl Engine {
    pub(super) fn chebyshev_call(
        &self,
        name: &str,
        args: &[Value],
        requested: usize,
    ) -> Result<Vec<Value>> {
        self.check_cancel()?;
        if !(3..=4).contains(&args.len())
            || !(1..=3).contains(&requested)
            || (name == "cheby1_sos" && requested != 1)
        {
            return Err("cheby1 accepts n,Rp,Wp[,type] with one/two b,a or three z,p,k outputs; cheby1_sos returns one SOS matrix. Analog/state-space/CTF forms are not implemented".into());
        }
        let n = scalar(&args[0])?;
        if !n.is_finite() || n.fract() != 0. || !(1.0..=dsp::MAX_CHEBY1_ORDER as f64).contains(&n) {
            return Err("cheby1 prototype order must be an integer in 1..20".into());
        }
        let ripple = scalar(&args[1])?;
        let edges = match &args[2] {
            Value::Matrix(a) if (a.rows() == 1 || a.cols() == 1) && (1..=2).contains(&a.len()) => {
                a.data()
            }
            _ => {
                return Err(
                    "cheby1 passband edges require a real scalar or two-element vector".into(),
                );
            }
        };
        // Bound optional text metadata before UTF16 decoding; invalid graphics,
        // cells and large aliased character arrays never enter generic coercion.
        let text = match args.get(3) {
            None => None,
            Some(Value::Text(s) | Value::QuotedString(s)) if s.len() <= 16 => Some(s.clone()),
            Some(Value::Container(Array::Character(a))) if a.len() <= 16 => {
                Some(a.to_utf8(&self.cancel)?)
            }
            _ => return Err("cheby1 type must be short text: low, high, bandpass or stop".into()),
        };
        let band = match text {
            Some(text) => Band::parse(&text.to_ascii_lowercase())?,
            None if edges.len() == 2 => Band::Bandpass,
            None => Band::Lowpass,
        };
        let paired = matches!(band, Band::Bandpass | Band::Bandstop);
        let degree = n as usize * if paired { 2 } else { 1 };
        let units = if name == "cheby1_sos" {
            degree.div_ceil(2) * 6
        } else if requested == 3 {
            4 * degree + 1
        } else {
            requested * (degree + 1)
        };
        if units > (32usize * 1024 * 1024).saturating_sub(self.evaluated_elements) {
            return Err("cheby1 output exceeds statement evaluation budget".into());
        }
        let design = dsp::cheby1(n as usize, ripple, edges, band, &self.cancel)?;
        let result = if name == "cheby1_sos" {
            vec![Value::Matrix(design.sos(&self.cancel)?)]
        } else if requested == 3 {
            let (z, p, k) = design.zero_pole_gain(&self.cancel)?;
            vec![output(z)?, output(p)?, Value::Matrix(Matrix::scalar(k))]
        } else {
            let (b, a) = design.transfer_function(&self.cancel)?;
            let mut values = vec![Value::Matrix(b)];
            if requested == 2 {
                values.push(Value::Matrix(a));
            }
            values
        };
        self.check_cancel()?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn budget_and_invalid_alias_arguments_fail_before_design_or_text_projection() {
        let mut e = Engine::default();
        let ch = crate::CharacterRow::from_units(vec![0xd800; 65536], &e.cancel).unwrap();
        let value = Value::Container(Array::Character(ch.clone()));
        assert!(
            e.chebyshev_call("cheby1", &vec![value; 64], 1)
                .unwrap_err()
                .contains("accepts")
        );
        e.evaluated_elements = 32 * 1024 * 1024;
        let args = [
            Value::Matrix(Matrix::scalar(3.)),
            Value::Matrix(Matrix::scalar(1.)),
            Value::Matrix(Matrix::scalar(0.3)),
        ];
        assert!(
            e.chebyshev_call("cheby1", &args, 2)
                .unwrap_err()
                .contains("budget")
        );
        assert_eq!(ch.units()[0], 0xd800);
    }
}
