//! Bounded value comparison, including handles nested in structs and cells.
use super::{Engine, Value};
use crate::{Array, Complex64, LogicalMatrix, Result};

fn number(array: &Array, index: usize) -> Complex64 {
    match array {
        Array::Numeric(a) => Complex64::new(a.data()[index], 0.),
        Array::Logical(a) => Complex64::new(f64::from(a.data()[index]), 0.),
        Array::Complex(a) => a.data()[index],
        Array::Character(a) => Complex64::new(f64::from(a.units()[index]), 0.),
        Array::Typed(a) => Complex64::new(a.real(index), 0.),
        _ => unreachable!("numeric or character values admitted before comparison"),
    }
}

impl Engine {
    pub(super) fn isequal_call(&mut self, args: &[Value]) -> Result<Value> {
        if args.len() < 2 {
            return Err("Not enough input arguments.".into());
        }
        let first = args[0].clone().array(&self.cancel)?;
        for value in &args[1..] {
            let other = value.clone().array(&self.cancel)?;
            if !self.equal_arrays(&first, &other)? {
                return Ok(Value::Logical(LogicalMatrix::scalar(false)));
            }
        }
        Ok(Value::Logical(LogicalMatrix::scalar(true)))
    }

    pub(super) fn equal_arrays(&mut self, a: &Array, b: &Array) -> Result<bool> {
        self.check_cancel()?;
        if let Some((a, b)) = super::objects::fis_comparison(self, a, b) {
            if self.functions.classes.equal_depth >= 64 {
                return Err("isequal: fuzzy object nesting is too deep".into());
            }
            self.functions.classes.equal_depth += 1;
            let equal = super::fuzzy::equal(&a, &b, |a, b| self.equal_arrays(a, b));
            self.functions.classes.equal_depth -= 1;
            return equal;
        }
        if let Some((a, b)) = super::objects::nnet_comparison(self, a, b)? {
            if self.functions.classes.equal_depth >= 64 {
                return Err("isequal: network nesting is too deep".into());
            }
            self.functions.classes.equal_depth += 1;
            let equal = self.equal_arrays(&a, &b);
            self.functions.classes.equal_depth -= 1;
            return equal;
        }
        if let Some(equal) = super::classdef::enum_equal(a, b) {
            return Ok(equal);
        }
        if (a.rows(), a.cols()) != (b.rows(), b.cols()) {
            return Ok(false);
        }
        // User objects: same class (a hidden field) and every property equal.
        if super::classdef::is_object(a) != super::classdef::is_object(b)
            || super::classdef::object_class(a) != super::classdef::object_class(b)
        {
            return Ok(false);
        }
        // Handle objects: the same object, or equal stored properties.
        match self.cls_handles_equal(a, b) {
            Some(super::classdef::HandleEquality::Decided(equal)) => return Ok(equal),
            Some(super::classdef::HandleEquality::Compare(x, y)) => {
                if self.functions.classes.equal_depth >= 64 {
                    return Err(
                        "isequal: handle objects refer to each other more than 64 levels deep."
                            .into(),
                    );
                }
                self.functions.classes.equal_depth += 1;
                let equal = self.equal_arrays(&x, &y);
                self.functions.classes.equal_depth -= 1;
                return equal;
            }
            None => {}
        }
        // Nested struct/cell comparisons bypass the top-level sparse dispatch.
        // Reuse its CSC equality kernel and charge stored entries, not numel.
        if let (Array::Sparse(a), Array::Sparse(b)) = (a, b) {
            self.charge(a.nnz().max(b.nnz()).max(1))?;
            let equal = a.equals(b, false);
            self.check_cancel()?;
            return Ok(equal);
        }
        self.charge(a.len().max(1))?;
        match (a, b) {
            (Array::Str(a), Array::Str(b)) => {
                for (i, (a, b)) in a.data().iter().zip(b.data()).enumerate() {
                    if i.is_multiple_of(1024) {
                        self.check_cancel()?;
                    }
                    if a.is_none() || a != b {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            (Array::Categorical(a), Array::Categorical(b)) => {
                super::categorical_fns::equal(a, b, false, &self.cancel)
            }
            (a @ Array::Datetime(_), b @ Array::Datetime(_))
            | (a @ Array::Duration(_), b @ Array::Duration(_)) => {
                Ok(super::datetime_fns::arrays_equal(a, b, false, &self.cancel)?.unwrap_or(false))
            }
            (Array::Table(a), Array::Table(b)) => {
                if a.names() != b.names()
                    || a.row_names() != b.row_names()
                    || a.is_timetable() != b.is_timetable()
                {
                    return Ok(false);
                }
                if let (Some(x), Some(y)) = (a.row_times(), b.row_times())
                    && !self.equal_arrays(x, y)?
                {
                    return Ok(false);
                }
                for (x, y) in a.columns().iter().zip(b.columns()) {
                    if !self.equal_arrays(x, y)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            (Array::Handle(a), Array::Handle(b)) => {
                Ok(a.same_storage(b) || a.name().is_some_and(|name| b.name() == Some(name)))
            }
            (Array::Cell(a), Array::Cell(b)) => {
                for i in 0..a.len() {
                    if !self.equal_arrays(a.get_linear(i)?, b.get_linear(i)?)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            (Array::Struct(a), Array::Struct(b)) => {
                if a.field_count() != b.field_count() {
                    return Ok(false);
                }
                for (field, name) in a.field_names().iter().enumerate() {
                    self.check_cancel()?;
                    let Some(other_field) = b.field_index(name) else {
                        return Ok(false);
                    };
                    for element in 0..a.len() {
                        if !self
                            .equal_arrays(a.get(field, element)?, b.get(other_field, element)?)?
                        {
                            return Ok(false);
                        }
                    }
                }
                Ok(true)
            }
            (Array::Graphics(a), Array::Graphics(b)) => {
                for i in 0..a.len() {
                    if i.is_multiple_of(1024) {
                        self.check_cancel()?;
                    }
                    if !a.get_linear(i)?.same_object(b.get_linear(i)?) {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            (a, b)
                if (a.is_numeric_array() || matches!(a, Array::Character(_) | Array::Typed(_)))
                    && (b.is_numeric_array()
                        || matches!(b, Array::Character(_) | Array::Typed(_))) =>
            {
                // Compare contents even for shared storage: a NaN is unequal
                // to itself. Real and imaginary components compare separately.
                for i in 0..a.len() {
                    if i.is_multiple_of(1024) {
                        self.check_cancel()?;
                    }
                    let equal = match (a, b) {
                        (Array::Typed(x), Array::Typed(y)) if x.is_integer() && y.is_integer() => {
                            x.int(i) == y.int(i)
                        }
                        (Array::Typed(x), y) | (y, Array::Typed(x)) if x.is_integer() => {
                            let y = number(y, i);
                            y.im == 0.
                                && super::typed_values::int_vs_real(x.int(i), y.re)
                                    == Some(std::cmp::Ordering::Equal)
                        }
                        _ => number(a, i) == number(b, i),
                    };
                    if !equal {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            _ => Ok(false),
        }
    }
}
