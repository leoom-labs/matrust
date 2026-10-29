//! Shape metadata queries for the engine's admitted two-dimensional arrays.
use super::{Engine, Value};
use crate::{LogicalMatrix, Matrix, Result};

impl Engine {
    pub(super) fn shape_query_call(&self, name: &str, args: &[Value]) -> Result<Option<Value>> {
        if !matches!(
            name,
            "isscalar" | "isvector" | "isrow" | "iscolumn" | "ismatrix" | "ndims"
        ) {
            return Ok(None);
        }
        self.check_cancel()?;
        let [value] = args else {
            return Err(format!("{name} requires exactly one array input"));
        };
        if matches!(value, Value::QuotedString(_) | Value::Void) {
            return Err(format!(
                "{name} requires an admitted array; MATLAB string arrays are not implemented"
            ));
        }
        // Stored values borrow rows/cols only, including malformed UTF16 and
        // deleted graphics handles. Raw character literals use the existing
        // bounded, cancellable UTF16-width scan without allocating a payload.
        let (rows, cols) = value.shape(&self.cancel)?;
        let result = match name {
            "ndims" => Value::Matrix(Matrix::scalar(2.)),
            _ => Value::Logical(LogicalMatrix::scalar(match name {
                "isscalar" => rows == 1 && cols == 1,
                "isvector" => rows == 1 || cols == 1,
                "isrow" => rows == 1,
                "iscolumn" => cols == 1,
                "ismatrix" => true,
                _ => unreachable!("shape query name admitted above"),
            })),
        };
        Ok(Some(result))
    }
}
