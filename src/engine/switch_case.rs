//! `switch` selection: scalar equality for numbers, exact text for character
//! rows, and any-element matching for cell cases. First match wins.
use super::{Engine, Execution};
use crate::{
    Array, Result, Scalar,
    syntax::{Expr, Statement, SwitchCase},
};

fn same_number(a: Scalar, b: Scalar) -> bool {
    if a.is_complex() || b.is_complex() {
        a.into_complex() == b.into_complex()
    } else {
        matches!((a.into_real(), b.into_real()), (Ok(a), Ok(b)) if a == b)
    }
}

fn same_value(subject: &Array, case: &Array) -> bool {
    match (subject, case) {
        (Array::Character(a), Array::Character(b)) => {
            a.rows() == b.rows() && a.cols() == b.cols() && a.units() == b.units()
        }
        // One character against a number compares by code: 'a' matches 97.
        (Array::Character(c), number) | (number, Array::Character(c)) => {
            let [code] = c.units() else { return false };
            number.is_numeric_array()
                && number.len() == 1
                && number
                    .scalar(0, 0)
                    .is_ok_and(|n| same_number(Scalar::Real(f64::from(*code)), n))
        }
        (a, b) if a.is_numeric_array() && b.is_numeric_array() && a.len() == 1 && b.len() == 1 => {
            matches!((a.scalar(0, 0), b.scalar(0, 0)), (Ok(a), Ok(b)) if same_number(a, b))
        }
        _ => false,
    }
}

impl Engine {
    /// Body of the first matching case, else the `otherwise` body. Case
    /// expressions after the match are never evaluated.
    pub(super) fn select_switch<'a>(
        &mut self,
        subject: &Expr,
        cases: &'a [SwitchCase],
        otherwise: &'a [Statement],
        byte: &mut usize,
        out: &mut Execution,
    ) -> Result<&'a [Statement]> {
        let subject = self.eval(subject, None, out)?.array(&self.cancel)?;
        let valid = match &subject {
            Array::Character(text) => text.rows() <= 1,
            other => other.is_numeric_array() && other.len() == 1,
        };
        if !valid {
            return Err("SWITCH expression must be a scalar or string constant.".into());
        }
        for case in cases {
            self.control_step(case.byte, byte)?;
            let candidate = self.eval(&case.value, None, out)?.array(&self.cancel)?;
            let matched = if let Array::Cell(cells) = &candidate {
                let mut matched = false;
                for i in 0..cells.len() {
                    if i.is_multiple_of(1024) {
                        self.check_cancel()?;
                    }
                    if same_value(&subject, cells.get_linear(i)?) {
                        matched = true;
                        break;
                    }
                }
                matched
            } else {
                same_value(&subject, &candidate)
            };
            if matched {
                return Ok(&case.body);
            }
        }
        Ok(otherwise)
    }
}
