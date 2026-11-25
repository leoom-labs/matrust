//! Data types for 2-D PDE coefficients, boundary conditions, and MATLAB struct views.
use crate::engine::Value;
use crate::{Array, CellArray, FunctionHandle, Matrix, Result, StructArray};
use std::sync::Arc;

/// PDE coefficient or boundary condition value: constant (scalar or column vector)
/// or `@(location, state)` function handle.
#[derive(Clone, Debug)]
pub(crate) enum CoeffVal {
    Const(Vec<f64>),
    Handle(FunctionHandle),
}

impl CoeffVal {
    pub(crate) fn scalar(v: f64) -> Self {
        Self::Const(vec![v])
    }

    pub(crate) fn is_zero_const(&self) -> bool {
        matches!(self, Self::Const(v) if v.iter().all(|&x| x == 0.0))
    }

    pub(crate) fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + match self {
                Self::Const(v) => v.len() * std::mem::size_of::<f64>(),
                Self::Handle(h) => h.bytes(),
            }
    }

    pub(crate) fn push_children<'a>(&'a self, pending: &mut Vec<&'a Array>) {
        if let Self::Handle(h) = self
            && let Some((_, captured)) = h.anonymous_parts()
        {
            pending.extend(captured.iter().map(|(_, a)| a));
        }
    }

    pub(crate) fn to_array(&self) -> Result<Array> {
        match self {
            Self::Const(v) if v.len() == 1 => Ok(Array::Numeric(Matrix::scalar(v[0]))),
            Self::Const(v) => Ok(Array::Numeric(Matrix::from_column_vec(
                v.len(),
                1,
                v.clone(),
            )?)),
            Self::Handle(h) => Ok(Array::Handle(h.clone())),
        }
    }
}

/// Expand a vector coefficient (`f`, `g`, `u`, `r`) of length `1` or `n` into a length-`n` vector.
pub(crate) fn expand_vec_coeff(vals: &[f64], n: usize, label: &str) -> Result<Vec<f64>> {
    if vals.len() == n {
        Ok(vals.to_vec())
    } else if vals.len() == 1 {
        Ok(vec![vals[0]; n])
    } else {
        Err(format!(
            "PDE {label} must have 1 or {n} rows for a {n}-equation system (got {})",
            vals.len()
        ))
    }
}

/// Expand a matrix coefficient (`m`, `d`, `a`, `q`) of length `1`, `n`, `n(n+1)/2`, or `n^2`
/// into a column-major `n x n` matrix (`out[j * n + i] = A_{i,j}`).
pub(crate) fn expand_mat_coeff(vals: &[f64], n: usize, label: &str) -> Result<Vec<f64>> {
    let mut out = vec![0.0; n * n];
    let len = vals.len();
    if len == 1 {
        let v = vals[0];
        for i in 0..n {
            out[i * n + i] = v;
        }
        return Ok(out);
    }
    if len == n {
        for i in 0..n {
            out[i * n + i] = vals[i];
        }
        return Ok(out);
    }
    let sym_len = n * (n + 1) / 2;
    if len == sym_len {
        let mut idx = 0;
        for j in 0..n {
            for i in 0..=j {
                let v = vals[idx];
                idx += 1;
                out[j * n + i] = v;
                out[i * n + j] = v;
            }
        }
        return Ok(out);
    }
    if len == n * n {
        out.copy_from_slice(vals);
        return Ok(out);
    }
    Err(format!(
        "PDE {label} must have 1, {n}, {sym_len}, or {} rows for a {n}-equation system (got {len})",
        n * n
    ))
}

/// Check whether a matrix coefficient row count is valid for an `n`-equation system.
pub(crate) fn valid_mat_coeff_len(len: usize, n: usize) -> bool {
    len == 1 || len == n || len == n * (n + 1) / 2 || len == n * n
}

/// Check whether a 2-D diffusion `c` coefficient row count is valid for an `n`-equation system.
pub(crate) fn valid_c_coeff_len(len: usize, n: usize) -> bool {
    if n == 1 {
        (1..=4).contains(&len)
    } else {
        (1..=4).contains(&len)
            || len == n
            || len == 2 * n
            || len == 3 * n
            || len == 4 * n
            || len == n * (2 * n + 1)
            || len == 4 * n * n
            || len == n * n
    }
}

/// Diffusion tensor `c`: constant or `@(location, state)`.
#[derive(Clone, Debug)]
pub(crate) enum CoeffC {
    Const(Vec<f64>),
    Handle(FunctionHandle),
}

impl CoeffC {
    /// Expand a 1-, 2-, 3-, or 4-element MATLAB `c` vector into `[c11, c21, c12, c22]` for `N = 1`.
    pub(crate) fn expand_tensor(vals: &[f64]) -> Result<[f64; 4]> {
        match vals {
            [c] => Ok([*c, 0.0, 0.0, *c]),
            [c11, c22] => Ok([*c11, 0.0, 0.0, *c22]),
            [c11, c12, c22] => Ok([*c11, *c12, *c12, *c22]),
            [c11, c21, c12, c22] => Ok([*c11, *c21, *c12, *c22]),
            _ => Err(format!(
                "PDE coefficient 'c' must have 1, 2, 3, or 4 rows for a scalar 2-D PDE (got {})",
                vals.len()
            )),
        }
    }

    /// Expand a MATLAB 2-D `c` vector for an `n`-equation system into a column-major
    /// `2n x 2n` tensor matrix `C` where row `2*i + k` and col `2*j + l`
    /// (`k, l in {0: x, 1: y}`) is `c(i,j,k,l)`:
    /// `out[(2*j + l) * (2*n) + (2*i + k)]`.
    ///
    /// Row counts and layouts follow the MathWorks "c Coefficient for Systems" page
    /// (1-based `c(i,j,k,l)` -> vector row):
    /// - `1`: `c(i,i,1,1) = c(i,i,2,2) = c`;
    /// - `2`: `c(i,i,1,1) = c(1)`, `c(i,i,2,2) = c(2)` for every `i`;
    /// - `3`: `c(i,i,1,1) = c(1)`, `c(i,i,1,2) = c(i,i,2,1) = c(2)`, `c(i,i,2,2) = c(3)`;
    /// - `4`: block `[c(1) c(3); c(2) c(4)]` on every diagonal block;
    /// - `N`: `c(i,i,1,1) = c(i,i,2,2) = c(i)`; the 2-, 3-, 4-element forms win for N = 2, 3, 4;
    /// - `2N`: `c(i,i,1,1) = c(2i-1)`, `c(i,i,2,2) = c(2i)`; the 4-element form wins for N = 2;
    /// - `3N`: row `3i + k + l - 4`, symmetric diagonal blocks;
    /// - `4N`: row `4i + 2l + k - 6`, diagonal blocks;
    /// - `2N(2N+1)/2`: symmetric, `i < j`: row `2j^2 - 3j + 4i + 2l + k - 5`,
    ///   `i = j`: row `2i^2 + i + l + k - 4`;
    /// - `4N^2`: row `4N(j-1) + 4i + 2l + k - 6`;
    /// - `N^2` (only when no documented form has that length, i.e. `N >= 5`): isotropic
    ///   `N x N` blocks, `c(i,j,k,k) = c(N(j-1) + i)` (column-major).
    pub(crate) fn expand_tensor_sys(vals: &[f64], n: usize) -> Result<Vec<f64>> {
        if n == 1 {
            let t = Self::expand_tensor(vals)?;
            return Ok(t.to_vec());
        }
        let m = 2 * n;
        let mut out = vec![0.0; m * m];
        // 0-based block (i, j), entries c11, c21, c12, c22 (k fastest).
        let set_block =
            |out: &mut [f64], i: usize, j: usize, c11: f64, c21: f64, c12: f64, c22: f64| {
                let r0 = 2 * i;
                let c0 = 2 * j;
                out[c0 * m + r0] = c11;
                out[c0 * m + r0 + 1] = c21;
                out[(c0 + 1) * m + r0] = c12;
                out[(c0 + 1) * m + r0 + 1] = c22;
            };
        let len = vals.len();
        // Diagonal-block forms whose block is the same for every equation (1, 2, 3, 4 rows).
        // Checked before the N-element and 2N-element forms: MATLAB gives them precedence.
        let same_block = match vals {
            [c] => Some([*c, 0.0, 0.0, *c]),
            [c11, c22] => Some([*c11, 0.0, 0.0, *c22]),
            [c11, c12, c22] => Some([*c11, *c12, *c12, *c22]),
            [c11, c21, c12, c22] => Some([*c11, *c21, *c12, *c22]),
            _ => None,
        };
        if let Some([c11, c21, c12, c22]) = same_block {
            for i in 0..n {
                set_block(&mut out, i, i, c11, c21, c12, c22);
            }
            return Ok(out);
        }
        if len == n {
            for (i, &v) in vals.iter().enumerate() {
                set_block(&mut out, i, i, v, 0.0, 0.0, v);
            }
            return Ok(out);
        }
        if len == 2 * n {
            for i in 0..n {
                set_block(&mut out, i, i, vals[2 * i], 0.0, 0.0, vals[2 * i + 1]);
            }
            return Ok(out);
        }
        if len == 3 * n {
            for i in 0..n {
                let c11 = vals[3 * i];
                let c12 = vals[3 * i + 1];
                let c22 = vals[3 * i + 2];
                set_block(&mut out, i, i, c11, c12, c12, c22);
            }
            return Ok(out);
        }
        if len == 4 * n {
            for i in 0..n {
                set_block(
                    &mut out,
                    i,
                    i,
                    vals[4 * i],
                    vals[4 * i + 1],
                    vals[4 * i + 2],
                    vals[4 * i + 3],
                );
            }
            return Ok(out);
        }
        let sym_len = n * (2 * n + 1);
        if len == sym_len {
            // Block-wise upper triangle: for block column j, the off-diagonal blocks i < j
            // (4 entries each, k fastest), then the symmetric diagonal block (c11, c12, c22).
            let mut idx = 0;
            for j in 0..n {
                for i in 0..j {
                    let b = &vals[idx..idx + 4];
                    idx += 4;
                    set_block(&mut out, i, j, b[0], b[1], b[2], b[3]);
                    // Symmetric tensor: c(j,i,l,k) = c(i,j,k,l).
                    set_block(&mut out, j, i, b[0], b[2], b[1], b[3]);
                }
                let b = &vals[idx..idx + 3];
                idx += 3;
                set_block(&mut out, j, j, b[0], b[1], b[1], b[2]);
            }
            return Ok(out);
        }
        if len == 4 * n * n {
            // Blocks in column-major block order, each block column-major.
            for j in 0..n {
                for i in 0..n {
                    let b = &vals[4 * (n * j + i)..4 * (n * j + i) + 4];
                    set_block(&mut out, i, j, b[0], b[1], b[2], b[3]);
                }
            }
            return Ok(out);
        }
        if len == n * n {
            // Isotropic N x N blocks (column-major): C_{i,j} = vals[j*n + i] * I_2.
            for j in 0..n {
                for i in 0..n {
                    let v = vals[j * n + i];
                    set_block(&mut out, i, j, v, 0.0, 0.0, v);
                }
            }
            return Ok(out);
        }
        Err(format!(
            "PDE coefficient 'c' must have 1, 2, 3, 4, {n}, {}, {}, {}, {sym_len}, {}, or {} rows for a {n}-equation 2-D PDE (got {len})",
            2 * n,
            3 * n,
            4 * n,
            4 * n * n,
            n * n,
        ))
    }

    pub(crate) fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + match self {
                Self::Const(v) => v.len() * std::mem::size_of::<f64>(),
                Self::Handle(h) => h.bytes(),
            }
    }

    pub(crate) fn push_children<'a>(&'a self, pending: &mut Vec<&'a Array>) {
        if let Self::Handle(h) = self
            && let Some((_, captured)) = h.anonymous_parts()
        {
            pending.extend(captured.iter().map(|(_, a)| a));
        }
    }

    pub(crate) fn to_array(&self) -> Result<Array> {
        match self {
            Self::Const(v) if v.len() == 1 => Ok(Array::Numeric(Matrix::scalar(v[0]))),
            Self::Const(v) => Ok(Array::Numeric(Matrix::from_column_vec(
                v.len(),
                1,
                v.clone(),
            )?)),
            Self::Handle(h) => Ok(Array::Handle(h.clone())),
        }
    }
}

/// One `specifyCoefficients` call stored on `PdeModel`.
#[derive(Clone, Debug)]
pub(crate) struct CoeffAssignment {
    /// `None` means all faces; `Some(ids)` holds 1-based face IDs.
    pub(crate) faces: Option<Vec<usize>>,
    pub(crate) m: CoeffVal,
    pub(crate) d: CoeffVal,
    pub(crate) c: CoeffC,
    pub(crate) a: CoeffVal,
    pub(crate) f: CoeffVal,
}

impl CoeffAssignment {
    pub(crate) fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self
                .faces
                .as_ref()
                .map_or(0, |v| v.len() * std::mem::size_of::<usize>())
            + self.m.bytes()
            + self.d.bytes()
            + self.c.bytes()
            + self.a.bytes()
            + self.f.bytes()
    }

    pub(crate) fn push_children<'a>(&'a self, pending: &mut Vec<&'a Array>) {
        self.m.push_children(pending);
        self.d.push_children(pending);
        self.c.push_children(pending);
        self.a.push_children(pending);
        self.f.push_children(pending);
    }

    pub(crate) fn is_time_dependent(&self) -> bool {
        !self.m.is_zero_const() || !self.d.is_zero_const()
    }

    pub(crate) fn to_struct(&self, cancel: &std::sync::atomic::AtomicBool) -> Result<Array> {
        let region_type = Value::Text("Face".into()).array(cancel)?;
        let region_id = match &self.faces {
            Some(ids) => Array::Numeric(Matrix::from_column_vec(
                1,
                ids.len(),
                ids.iter().map(|&id| id as f64).collect(),
            )?),
            None => Array::Numeric(Matrix::filled(0, 0, 0.0)?),
        };
        let names = vec![
            "RegionType".to_owned(),
            "RegionID".to_owned(),
            "m".to_owned(),
            "d".to_owned(),
            "c".to_owned(),
            "a".to_owned(),
            "f".to_owned(),
        ];
        let slots = vec![
            Arc::new(region_type),
            Arc::new(region_id),
            Arc::new(self.m.to_array()?),
            Arc::new(self.d.to_array()?),
            Arc::new(self.c.to_array()?),
            Arc::new(self.a.to_array()?),
            Arc::new(self.f.to_array()?),
        ];
        Ok(Array::Struct(StructArray::from_slots(
            1, 1, names, slots, cancel,
        )?))
    }
}

/// Build the `model.EquationCoefficients` record struct from all assignments.
pub(crate) fn coeff_records_array(
    assignments: &[CoeffAssignment],
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<Array> {
    let mut cells = Vec::with_capacity(assignments.len());
    for a in assignments {
        cells.push(Arc::new(a.to_struct(cancel)?));
    }
    let cell_arr = Array::Cell(CellArray::from_shared(1, cells.len(), cells, cancel)?);
    Ok(Array::Struct(StructArray::from_slots(
        1,
        1,
        vec!["CoefficientAssignments".to_owned()],
        vec![Arc::new(cell_arr)],
        cancel,
    )?))
}

/// Boundary condition type for a 2-D PDE (scalar or N-equation system).
#[derive(Clone, Debug)]
pub(crate) enum BcKind {
    DirichletU {
        u: CoeffVal,
        /// Optional 1-based component indices (`EquationIndex`/`Equation`). `None` means all `1..=N`.
        eq_indices: Option<Vec<usize>>,
    },
    DirichletHR {
        h: CoeffVal,
        r: CoeffVal,
        eq_indices: Option<Vec<usize>>,
    },
    Neumann {
        g: CoeffVal,
        q: CoeffVal,
        eq_indices: Option<Vec<usize>>,
    },
    Mixed {
        u: Option<CoeffVal>,
        h: Option<CoeffVal>,
        r: Option<CoeffVal>,
        g: Option<CoeffVal>,
        q: Option<CoeffVal>,
        eq_indices: Option<Vec<usize>>,
    },
}

impl BcKind {
    pub(crate) fn bytes(&self) -> usize {
        let eq_bytes = |eq: &Option<Vec<usize>>| {
            eq.as_ref()
                .map_or(0, |v| v.len() * std::mem::size_of::<usize>())
        };
        std::mem::size_of::<Self>()
            + match self {
                Self::DirichletU { u, eq_indices } => u.bytes() + eq_bytes(eq_indices),
                Self::DirichletHR { h, r, eq_indices } => {
                    h.bytes() + r.bytes() + eq_bytes(eq_indices)
                }
                Self::Neumann { g, q, eq_indices } => g.bytes() + q.bytes() + eq_bytes(eq_indices),
                Self::Mixed {
                    u,
                    h,
                    r,
                    g,
                    q,
                    eq_indices,
                } => {
                    u.as_ref().map_or(0, CoeffVal::bytes)
                        + h.as_ref().map_or(0, CoeffVal::bytes)
                        + r.as_ref().map_or(0, CoeffVal::bytes)
                        + g.as_ref().map_or(0, CoeffVal::bytes)
                        + q.as_ref().map_or(0, CoeffVal::bytes)
                        + eq_bytes(eq_indices)
                }
            }
    }

    pub(crate) fn push_children<'a>(&'a self, pending: &mut Vec<&'a Array>) {
        match self {
            Self::DirichletU { u, .. } => u.push_children(pending),
            Self::DirichletHR { h, r, .. } => {
                h.push_children(pending);
                r.push_children(pending);
            }
            Self::Neumann { g, q, .. } => {
                g.push_children(pending);
                q.push_children(pending);
            }
            Self::Mixed { u, h, r, g, q, .. } => {
                if let Some(v) = u {
                    v.push_children(pending);
                }
                if let Some(v) = h {
                    v.push_children(pending);
                }
                if let Some(v) = r {
                    v.push_children(pending);
                }
                if let Some(v) = g {
                    v.push_children(pending);
                }
                if let Some(v) = q {
                    v.push_children(pending);
                }
            }
        }
    }
}

/// One `applyBoundaryCondition` call stored on `PdeModel`.
#[derive(Clone, Debug)]
pub(crate) struct BcAssignment {
    /// 1-based geometry edge IDs.
    pub(crate) edges: Vec<usize>,
    pub(crate) kind: BcKind,
}

impl BcAssignment {
    pub(crate) fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.edges.len() * std::mem::size_of::<usize>()
            + self.kind.bytes()
    }

    pub(crate) fn push_children<'a>(&'a self, pending: &mut Vec<&'a Array>) {
        self.kind.push_children(pending);
    }

    pub(crate) fn to_struct(&self, cancel: &std::sync::atomic::AtomicBool) -> Result<Array> {
        let region_type = Value::Text("Edge".into()).array(cancel)?;
        let region_id = Array::Numeric(Matrix::from_column_vec(
            1,
            self.edges.len(),
            self.edges.iter().map(|&id| id as f64).collect(),
        )?);
        let mut names = vec!["RegionType".to_owned(), "RegionID".to_owned()];
        let mut slots = vec![Arc::new(region_type), Arc::new(region_id)];
        match &self.kind {
            BcKind::DirichletU { u, eq_indices } => {
                names.push("u".to_owned());
                slots.push(Arc::new(u.to_array()?));
                if let Some(eq) = eq_indices {
                    names.push("EquationIndex".to_owned());
                    slots.push(Arc::new(Array::Numeric(Matrix::from_column_vec(
                        1,
                        eq.len(),
                        eq.iter().map(|&i| i as f64).collect(),
                    )?)));
                }
            }
            BcKind::DirichletHR { h, r, eq_indices } => {
                names.push("h".to_owned());
                names.push("r".to_owned());
                slots.push(Arc::new(h.to_array()?));
                slots.push(Arc::new(r.to_array()?));
                if let Some(eq) = eq_indices {
                    names.push("EquationIndex".to_owned());
                    slots.push(Arc::new(Array::Numeric(Matrix::from_column_vec(
                        1,
                        eq.len(),
                        eq.iter().map(|&i| i as f64).collect(),
                    )?)));
                }
            }
            BcKind::Neumann { g, q, eq_indices } => {
                names.push("g".to_owned());
                names.push("q".to_owned());
                slots.push(Arc::new(g.to_array()?));
                slots.push(Arc::new(q.to_array()?));
                if let Some(eq) = eq_indices {
                    names.push("EquationIndex".to_owned());
                    slots.push(Arc::new(Array::Numeric(Matrix::from_column_vec(
                        1,
                        eq.len(),
                        eq.iter().map(|&i| i as f64).collect(),
                    )?)));
                }
            }
            BcKind::Mixed {
                u,
                h,
                r,
                g,
                q,
                eq_indices,
            } => {
                if let Some(v) = u {
                    names.push("u".to_owned());
                    slots.push(Arc::new(v.to_array()?));
                }
                if let Some(v) = h {
                    names.push("h".to_owned());
                    slots.push(Arc::new(v.to_array()?));
                }
                if let Some(v) = r {
                    names.push("r".to_owned());
                    slots.push(Arc::new(v.to_array()?));
                }
                if let Some(v) = g {
                    names.push("g".to_owned());
                    slots.push(Arc::new(v.to_array()?));
                }
                if let Some(v) = q {
                    names.push("q".to_owned());
                    slots.push(Arc::new(v.to_array()?));
                }
                if let Some(eq) = eq_indices {
                    names.push("EquationIndex".to_owned());
                    slots.push(Arc::new(Array::Numeric(Matrix::from_column_vec(
                        1,
                        eq.len(),
                        eq.iter().map(|&i| i as f64).collect(),
                    )?)));
                }
            }
        }
        Ok(Array::Struct(StructArray::from_slots(
            1, 1, names, slots, cancel,
        )?))
    }
}

/// Build the `model.BoundaryConditions` record struct from all assignments.
pub(crate) fn bc_records_array(
    assignments: &[BcAssignment],
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<Array> {
    let mut cells = Vec::with_capacity(assignments.len());
    for a in assignments {
        cells.push(Arc::new(a.to_struct(cancel)?));
    }
    let cell_arr = Array::Cell(CellArray::from_shared(1, cells.len(), cells, cancel)?);
    Ok(Array::Struct(StructArray::from_slots(
        1,
        1,
        vec!["BoundaryConditionAssignments".to_owned()],
        vec![Arc::new(cell_arr)],
        cancel,
    )?))
}

/// Solver tolerances and nonlinear options stored on `model.SolverOptions`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SolverOptions {
    pub(crate) relative_tolerance: f64,
    pub(crate) absolute_tolerance: f64,
    pub(crate) residual_tolerance: f64,
    pub(crate) max_iterations: usize,
    pub(crate) min_step: f64,
    pub(crate) report_statistics: bool,
}

impl Default for SolverOptions {
    fn default() -> Self {
        Self {
            relative_tolerance: 1e-3,
            absolute_tolerance: 1e-6,
            residual_tolerance: 1e-4,
            max_iterations: 25,
            min_step: 1.0 / 65536.0,
            report_statistics: false,
        }
    }
}

impl SolverOptions {
    #[allow(clippy::wrong_self_convention)]
    pub(crate) fn to_struct(&self, cancel: &std::sync::atomic::AtomicBool) -> Result<Array> {
        let rep =
            Value::Text(if self.report_statistics { "on" } else { "off" }.into()).array(cancel)?;
        Ok(Array::Struct(StructArray::from_slots(
            1,
            1,
            vec![
                "RelativeTolerance".to_owned(),
                "AbsoluteTolerance".to_owned(),
                "ResidualTolerance".to_owned(),
                "MaxIterations".to_owned(),
                "MinStep".to_owned(),
                "ReportStatistics".to_owned(),
            ],
            vec![
                Arc::new(Array::Numeric(Matrix::scalar(self.relative_tolerance))),
                Arc::new(Array::Numeric(Matrix::scalar(self.absolute_tolerance))),
                Arc::new(Array::Numeric(Matrix::scalar(self.residual_tolerance))),
                Arc::new(Array::Numeric(Matrix::scalar(self.max_iterations as f64))),
                Arc::new(Array::Numeric(Matrix::scalar(self.min_step))),
                Arc::new(rep),
            ],
            cancel,
        )?))
    }
}

/// Initial condition value for `u0` or `ut0`.
#[derive(Clone, Debug)]
pub(crate) enum IcValue {
    Const(Vec<f64>),
    Handle(FunctionHandle),
    Nodal { data: Arc<Vec<f64>>, n_comp: usize },
}

impl IcValue {
    pub(crate) fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + match self {
                Self::Const(v) => v.len() * std::mem::size_of::<f64>(),
                Self::Handle(h) => h.bytes(),
                Self::Nodal { data, .. } => data.len() * std::mem::size_of::<f64>(),
            }
    }

    pub(crate) fn push_children<'a>(&'a self, pending: &mut Vec<&'a Array>) {
        if let Self::Handle(h) = self
            && let Some((_, captured)) = h.anonymous_parts()
        {
            pending.extend(captured.iter().map(|(_, a)| a));
        }
    }

    pub(crate) fn to_array(&self) -> Result<Array> {
        match self {
            Self::Const(v) if v.len() == 1 => Ok(Array::Numeric(Matrix::scalar(v[0]))),
            Self::Const(v) => Ok(Array::Numeric(Matrix::from_column_vec(
                v.len(),
                1,
                v.clone(),
            )?)),
            Self::Handle(h) => Ok(Array::Handle(h.clone())),
            Self::Nodal { data, n_comp } => {
                let n_nodes = data.len() / (*n_comp).max(1);
                Ok(Array::Numeric(Matrix::from_column_slice(
                    n_nodes, *n_comp, data,
                )?))
            }
        }
    }
}

/// One `setInitialConditions` call stored on `PdeModel`.
#[derive(Clone, Debug)]
pub(crate) struct IcAssignment {
    /// `None` means all faces / whole domain; `Some(ids)` holds 1-based face IDs.
    pub(crate) faces: Option<Vec<usize>>,
    pub(crate) u0: IcValue,
    pub(crate) ut0: Option<IcValue>,
}

impl IcAssignment {
    pub(crate) fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self
                .faces
                .as_ref()
                .map_or(0, |v| v.len() * std::mem::size_of::<usize>())
            + self.u0.bytes()
            + self.ut0.as_ref().map_or(0, IcValue::bytes)
    }

    pub(crate) fn push_children<'a>(&'a self, pending: &mut Vec<&'a Array>) {
        self.u0.push_children(pending);
        if let Some(ut0) = &self.ut0 {
            ut0.push_children(pending);
        }
    }

    pub(crate) fn to_struct(&self, cancel: &std::sync::atomic::AtomicBool) -> Result<Array> {
        let region_type = Value::Text("Face".into()).array(cancel)?;
        let region_id = match &self.faces {
            Some(ids) => Array::Numeric(Matrix::from_column_vec(
                1,
                ids.len(),
                ids.iter().map(|&id| id as f64).collect(),
            )?),
            None => Array::Numeric(Matrix::filled(0, 0, 0.0)?),
        };
        let ut0_arr = match &self.ut0 {
            Some(v) => v.to_array()?,
            None => Array::Numeric(Matrix::filled(0, 0, 0.0)?),
        };
        let names = vec![
            "RegionType".to_owned(),
            "RegionID".to_owned(),
            "InitialValue".to_owned(),
            "InitialDerivative".to_owned(),
        ];
        let slots = vec![
            Arc::new(region_type),
            Arc::new(region_id),
            Arc::new(self.u0.to_array()?),
            Arc::new(ut0_arr),
        ];
        Ok(Array::Struct(StructArray::from_slots(
            1, 1, names, slots, cancel,
        )?))
    }
}

/// Build the `model.InitialConditions` record struct from all assignments.
pub(crate) fn ic_records_array(
    assignments: &[IcAssignment],
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<Array> {
    let mut cells = Vec::with_capacity(assignments.len());
    for a in assignments {
        cells.push(Arc::new(a.to_struct(cancel)?));
    }
    let cell_arr = Array::Cell(CellArray::from_shared(1, cells.len(), cells, cancel)?);
    Ok(Array::Struct(StructArray::from_slots(
        1,
        1,
        vec!["InitialConditionAssignments".to_owned()],
        vec![Arc::new(cell_arr)],
        cancel,
    )?))
}

/// Extract a numeric real finite matrix from a `Value`.
pub(crate) fn numeric_matrix(v: &Value, label: &str) -> Result<Matrix> {
    let m = match v {
        Value::Matrix(m) | Value::Container(Array::Numeric(m)) => m.clone(),
        Value::Logical(l) | Value::Container(Array::Logical(l)) => {
            let data = l.data().iter().map(|&b| f64::from(b)).collect();
            Matrix::from_column_vec(l.rows(), l.cols(), data)?
        }
        Value::Container(Array::Typed(a)) => a.to_matrix()?,
        _ => return Err(format!("{label} must be a real numeric value")),
    };
    if m.is_nd() || m.data().iter().any(|x| !x.is_finite()) {
        return Err(format!("{label} must contain finite real 2-D values"));
    }
    Ok(m)
}

/// Extract text from a `Value`.
pub(crate) fn text_value(v: &Value) -> Option<String> {
    match v {
        Value::Text(s) | Value::QuotedString(s) => Some(s.clone()),
        Value::Container(Array::Character(c)) if c.rows() <= 1 => {
            Some(String::from_utf16_lossy(c.units()))
        }
        Value::Container(Array::Str(s)) if s.len() == 1 => {
            s.data()[0].as_deref().map(str::to_owned)
        }
        _ => None,
    }
}
